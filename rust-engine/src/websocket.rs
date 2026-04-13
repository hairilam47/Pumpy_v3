// WebSocket monitor for Pump.fun program events
// Subscribes to Solana program logs and account changes for the Pump.fun program

use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use tracing::{info, warn, error, debug};

use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;

use crate::rpc::RpcManager;
use crate::pumpfun::{PumpFunClient, TokenDiscoveredEvent};
use crate::metrics::Metrics;
use crate::constants::PUMPFUN_PROGRAM_ID;

pub struct WebSocketMonitor {
    rpc_manager: Arc<RpcManager>,
    pumpfun_client: Arc<PumpFunClient>,
    metrics: Arc<Metrics>,
}

impl WebSocketMonitor {
    pub fn new(
        rpc_manager: Arc<RpcManager>,
        pumpfun_client: Arc<PumpFunClient>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            rpc_manager,
            pumpfun_client,
            metrics,
        }
    }

    /// Start monitoring Pump.fun program for new token launches and trades
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ws_url = self.rpc_manager.get_websocket_url().await
            .unwrap_or_else(|| "wss://api.mainnet-beta.solana.com".to_string());

        info!("WebSocket monitor starting, connecting to {}", ws_url);

        let mut backoff = Duration::from_secs(1);
        loop {
            match self.connect_and_monitor(&ws_url).await {
                Ok(_) => {
                    info!("WebSocket disconnected, reconnecting in 1s...");
                    backoff = Duration::from_secs(1);
                }
                Err(e) => {
                    error!("WebSocket error: {}, retrying in {:?}", e, backoff);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                }
            }
        }
    }

    async fn connect_and_monitor(
        &self,
        ws_url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Connecting to Solana WebSocket: {}", ws_url);

        let pubsub = PubsubClient::new(ws_url).await
            .map_err(|e| format!("PubsubClient connect error: {e}"))?;

        info!(
            "WebSocket connected. Subscribing to Pump.fun program logs: {}",
            PUMPFUN_PROGRAM_ID
        );

        // Subscribe to Pump.fun program log messages
        // This receives events for every create/buy/sell instruction
        let (mut log_stream, _log_unsub) = pubsub
            .logs_subscribe(
                RpcTransactionLogsFilter::Mentions(vec![PUMPFUN_PROGRAM_ID.to_string()]),
                RpcTransactionLogsConfig {
                    commitment: Some(CommitmentConfig::confirmed()),
                },
            )
            .await
            .map_err(|e| format!("logsSubscribe error: {e}"))?;

        info!("Subscribed to Pump.fun program logs");

        loop {
            use futures_util::StreamExt;
            match tokio::time::timeout(Duration::from_secs(60), log_stream.next()).await {
                Ok(Some(response)) => {
                    let logs = &response.value.logs;
                    let sig = &response.value.signature;
                    debug!("Got log event for tx: {}", sig);

                    self.metrics.tokens_discovered.inc();

                    // Parse the log lines to identify event type
                    if self.is_token_create(logs) {
                        if let Some(event) = self.parse_create_event(logs, sig) {
                            info!(
                                "New token discovered: {} ({}) via tx {}",
                                event.name, event.symbol, sig
                            );
                            self.metrics.tokens_discovered.inc();
                            let _ = self.pumpfun_client.publish_token_event(event);
                        }
                    } else if self.is_buy_event(logs) {
                        debug!("Buy event detected in tx {}", sig);
                    } else if self.is_sell_event(logs) {
                        debug!("Sell event detected in tx {}", sig);
                    }
                }
                Ok(None) => {
                    warn!("WebSocket log stream ended");
                    break;
                }
                Err(_) => {
                    warn!("WebSocket heartbeat timeout, reconnecting...");
                    break;
                }
            }
        }

        Ok(())
    }

    fn is_token_create(&self, logs: &[String]) -> bool {
        logs.iter().any(|l| l.contains("Create") || l.contains("InitializeMint"))
    }

    fn is_buy_event(&self, logs: &[String]) -> bool {
        logs.iter().any(|l| l.contains("Buy") && l.contains("Program log:"))
    }

    fn is_sell_event(&self, logs: &[String]) -> bool {
        logs.iter().any(|l| l.contains("Sell") && l.contains("Program log:"))
    }

    /// Parse a token creation event from program log lines.
    /// Pump.fun uses Anchor's event macros so logs contain base64-encoded event data.
    fn parse_create_event(&self, logs: &[String], sig: &str) -> Option<TokenDiscoveredEvent> {
        // Pump.fun log format for create events:
        //   "Program log: Instruction: Create"
        //   "Program log: <base64-encoded event data>"
        // We extract what we can from the log lines; full parsing requires
        // Anchor IDL deserialization which is handled by the Python strategy engine.

        let has_create = logs.iter().any(|l| l.contains("Instruction: Create"));
        if !has_create {
            return None;
        }

        // Extract mint from the event data if present
        // The event data is base64-encoded Anchor event struct
        let mint_str = logs
            .iter()
            .find(|l| l.starts_with("Program data: "))
            .and_then(|l| {
                let b64 = l.trim_start_matches("Program data: ");
                base64::decode(b64).ok().and_then(|bytes| {
                    // Skip 8-byte discriminator, then parse pubkey (32 bytes)
                    if bytes.len() >= 40 {
                        let mint_bytes: [u8; 32] = bytes[8..40].try_into().ok()?;
                        Some(Pubkey::new_from_array(mint_bytes).to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("unknown-{}", &sig[..8]));

        Some(TokenDiscoveredEvent {
            mint: mint_str,
            name: "New Token".to_string(),
            symbol: "NEW".to_string(),
            uri: String::new(),
            creator: String::new(),
            bonding_curve: String::new(),
            timestamp: chrono::Utc::now().timestamp(),
            virtual_sol_reserves: 30_000_000_000, // 30 SOL initial
            virtual_token_reserves: 1_073_000_000_000_000, // typical initial supply
        })
    }
}
