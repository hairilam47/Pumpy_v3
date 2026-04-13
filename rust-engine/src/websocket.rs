// WebSocket monitor for Pump.fun program events
// Subscribes to both:
//   1. logsSubscribe  — receives every transaction that mentions the Pump.fun program
//   2. programSubscribe — receives every account owned by the Pump.fun program (bonding curves etc.)

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn, error, debug};

use solana_sdk::pubkey::Pubkey;

use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::{
    RpcTransactionLogsConfig, RpcTransactionLogsFilter,
    RpcProgramAccountsConfig, RpcAccountInfoConfig,
};
use solana_account_decoder::UiAccountEncoding;
use solana_sdk::commitment_config::CommitmentConfig;

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

        // Spawn both subscription tasks concurrently; exit when either one ends.
        let log_future = self.run_logs_subscription(ws_url);
        let account_future = self.run_program_account_subscription(ws_url);

        tokio::select! {
            res = log_future => {
                if let Err(e) = res { warn!("logsSubscribe ended: {}", e); }
            }
            res = account_future => {
                if let Err(e) = res { warn!("programSubscribe ended: {}", e); }
            }
        }

        Ok(())
    }

    /// Subscribe to transaction logs for the Pump.fun program (logsSubscribe).
    /// Fires on every buy/sell/create instruction.
    async fn run_logs_subscription(
        &self,
        ws_url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let pubsub = PubsubClient::new(ws_url).await
            .map_err(|e| format!("PubsubClient (logs) connect error: {e}"))?;

        let (mut stream, _unsub) = pubsub
            .logs_subscribe(
                RpcTransactionLogsFilter::Mentions(vec![PUMPFUN_PROGRAM_ID.to_string()]),
                RpcTransactionLogsConfig {
                    commitment: Some(CommitmentConfig::confirmed()),
                },
            )
            .await
            .map_err(|e| format!("logsSubscribe error: {e}"))?;

        info!("Subscribed to Pump.fun program logs (logsSubscribe)");

        loop {
            use futures_util::StreamExt;
            match tokio::time::timeout(Duration::from_secs(60), stream.next()).await {
                Ok(Some(response)) => {
                    let logs = &response.value.logs;
                    let sig = &response.value.signature;
                    debug!("Log event for tx: {}", sig);

                    if self.is_token_create(logs) {
                        if let Some(event) = self.parse_create_event(logs, sig) {
                            info!("New token discovered via logs: {} ({})", event.name, event.symbol);
                            self.metrics.tokens_discovered.inc();
                            let _ = self.pumpfun_client.publish_token_event(event);
                        }
                    } else if self.is_buy_event(logs) {
                        debug!("Buy event in tx {}", sig);
                    } else if self.is_sell_event(logs) {
                        debug!("Sell event in tx {}", sig);
                    }
                }
                Ok(None) => { warn!("Logs stream ended"); break; }
                Err(_) => { warn!("Logs stream heartbeat timeout"); break; }
            }
        }
        Ok(())
    }

    /// Subscribe to account updates for all accounts owned by the Pump.fun program
    /// (programSubscribe).  Fires whenever a bonding curve account is created or
    /// updated — this catches new token launches at the account level.
    async fn run_program_account_subscription(
        &self,
        ws_url: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let program_pubkey = PUMPFUN_PROGRAM_ID;

        let pubsub = PubsubClient::new(ws_url).await
            .map_err(|e| format!("PubsubClient (program) connect error: {e}"))?;

        let config = RpcProgramAccountsConfig {
            account_config: RpcAccountInfoConfig {
                encoding: Some(UiAccountEncoding::Base64),
                commitment: Some(CommitmentConfig::confirmed()),
                ..Default::default()
            },
            filters: None,
            with_context: Some(true),
            sort_results: Some(false),
        };

        let (mut stream, _unsub) = pubsub
            .program_subscribe(&program_pubkey, Some(config))
            .await
            .map_err(|e| format!("programSubscribe error: {e}"))?;

        info!("Subscribed to Pump.fun program accounts (programSubscribe)");

        loop {
            use futures_util::StreamExt;
            match tokio::time::timeout(Duration::from_secs(120), stream.next()).await {
                Ok(Some(response)) => {
                    let account_key = response.value.pubkey.clone();
                    debug!("Program account update: {}", account_key);
                    // Account data changes signal bonding curve activity.
                    // We emit a lightweight event so downstream strategies can react.
                    let event = TokenDiscoveredEvent {
                        mint: account_key.clone(),
                        name: "Unknown".to_string(),
                        symbol: "UNK".to_string(),
                        uri: String::new(),
                        creator: String::new(),
                        bonding_curve: account_key,
                        timestamp: chrono::Utc::now().timestamp(),
                        virtual_sol_reserves: 30_000_000_000,
                        virtual_token_reserves: 1_073_000_000_000_000,
                    };
                    let _ = self.pumpfun_client.publish_token_event(event);
                }
                Ok(None) => { warn!("Program account stream ended"); break; }
                Err(_) => { warn!("Program account stream heartbeat timeout"); break; }
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
