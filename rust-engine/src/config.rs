use std::env;
use serde_json;

fn provider_name(url: &str) -> String {
    if url.contains("helius") { "helius".to_string() }
    else if url.contains("quicknode") { "quicknode".to_string() }
    else if url.contains("alchemy") { "alchemy".to_string() }
    else { "public".to_string() }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub environment: String,
    pub database_url: String,
    pub redis_url: String,
    pub grpc_port: u16,
    pub metrics_port: u16,
    pub keypair_bytes: Vec<u8>,
    pub rpc_endpoints: Vec<RpcEndpointConfig>,
    pub jito_bundle_url: Option<String>,
    pub execution_workers: usize,
    pub max_concurrent_orders: usize,
    pub order_timeout_seconds: u64,
    pub risk_limits: RiskLimits,
    pub trading: TradingConfig,
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Clone)]
pub struct RpcEndpointConfig {
    pub url: String,
    pub provider: String,
    pub priority: u8,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RiskLimits {
    pub max_position_size_sol: f64,
    pub max_portfolio_exposure_sol: f64,
    pub max_daily_loss_sol: f64,
    pub max_slippage_bps: u64,
    pub max_sandwich_risk_score: u32,
}

#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub default_slippage_bps: u64,
    pub min_trade_size_sol: f64,
    pub max_trade_size_sol: f64,
    pub mev_protection_enabled: bool,
    pub tip_percentage: f64,
    pub retry_attempts: u32,
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    pub log_level: String,
    pub slack_webhook_url: Option<String>,
}

impl Config {
    /// Apply non-sensitive overrides loaded from the bot_config DB table.
    /// DB values take precedence over env vars. Best-effort — unknown keys are ignored.
    ///
    /// RPC precedence (same as from_env but sourced from DB):
    ///   DB SOLANA_RPC_URL  → primary endpoint
    ///   DB SOLANA_RPC_URLS → comma-separated failover list
    ///   If only one is set, env-derived counterpart provides the missing half.
    pub fn apply_db_overrides(&mut self, overrides: &std::collections::HashMap<String, String>) {
        // ── RPC endpoints ─────────────────────────────────────────────────────
        let db_primary = overrides.get("SOLANA_RPC_URL").filter(|v| !v.is_empty()).cloned();
        let db_failover: Option<Vec<String>> = overrides
            .get("SOLANA_RPC_URLS")
            .filter(|v| !v.is_empty())
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

        if db_primary.is_some() || db_failover.is_some() {
            // Primary: DB value, or fall back to the env-derived first endpoint
            let env_primary = self
                .rpc_endpoints
                .first()
                .map(|e| e.url.clone())
                .unwrap_or_else(|| "https://api.mainnet-beta.solana.com".to_string());
            let primary_url = db_primary.unwrap_or(env_primary);

            // Failover: DB list (minus duplicates of primary), or env-derived remainder
            let failover_urls: Vec<String> = match db_failover {
                Some(db_fo) => db_fo.into_iter().filter(|u| u != &primary_url).collect(),
                None => self.rpc_endpoints.iter().skip(1).map(|e| e.url.clone()).collect(),
            };

            let mut endpoints = vec![RpcEndpointConfig {
                url: primary_url.clone(),
                provider: provider_name(&primary_url),
                priority: 1,
                ws_url: None,
            }];
            for (i, url) in failover_urls.into_iter().enumerate() {
                endpoints.push(RpcEndpointConfig {
                    url: url.clone(),
                    provider: provider_name(&url),
                    priority: (i + 2) as u8,
                    ws_url: None,
                });
            }
            let total = endpoints.len();
            self.rpc_endpoints = endpoints;
            tracing::info!(
                "bot_config: RPC endpoints rebuilt from DB (primary={}, total={})",
                primary_url,
                total
            );
        }

        // ── Jito bundle URL ────────────────────────────────────────────────────
        if let Some(url) = overrides.get("JITO_BUNDLE_URL").filter(|v| !v.is_empty()) {
            self.jito_bundle_url = Some(url.clone());
            tracing::info!("bot_config: JITO_BUNDLE_URL overridden from DB");
        }

        // ── Risk limits ────────────────────────────────────────────────────────
        if let Some(v) = overrides.get("MAX_POSITION_SIZE_SOL") {
            if let Ok(f) = v.parse::<f64>() {
                self.risk_limits.max_position_size_sol = f;
                tracing::info!("bot_config: MAX_POSITION_SIZE_SOL overridden from DB ({})", f);
            }
        }
    }

    pub fn from_env() -> Result<Self, String> {
        // RPC endpoint resolution (priority order):
        //   1. SOLANA_RPC_URL  — canonical single endpoint (simple path, set this first)
        //   2. SOLANA_RPC_URLS — comma-separated failover list (used when SOLANA_RPC_URL absent)
        //   When both are set, SOLANA_RPC_URL is the primary and SOLANA_RPC_URLS entries are
        //   appended as additional failover candidates.
        //   Optional companion vars: SOLANA_WS_URLS, RPC_PRIORITIES, RPC_PROVIDERS
        let rpc_endpoints = {
            let urls: Vec<String> = {
                let single = env::var("SOLANA_RPC_URL").ok();
                let multi: Vec<String> = env::var("SOLANA_RPC_URLS")
                    .ok()
                    .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                    .unwrap_or_default();
                match single {
                    Some(url) => {
                        // SOLANA_RPC_URL is canonical — always first; SOLANA_RPC_URLS are failover
                        let mut list = vec![url.trim().to_string()];
                        list.extend(multi);
                        list
                    }
                    None if !multi.is_empty() => multi,
                    _ => vec!["https://api.mainnet-beta.solana.com".to_string()],
                }
            };

            let ws_urls: Vec<Option<String>> = env::var("SOLANA_WS_URLS")
                .ok()
                .map(|v| v.split(',').map(|s| {
                    let s = s.trim().to_string();
                    if s.is_empty() { None } else { Some(s) }
                }).collect())
                .unwrap_or_else(|| {
                    vec![env::var("SOLANA_WS_URL").ok()]
                });

            let priorities: Vec<u8> = env::var("RPC_PRIORITIES")
                .ok()
                .map(|v| v.split(',').enumerate().map(|(i, s)| {
                    s.trim().parse::<u8>().unwrap_or((i + 1) as u8)
                }).collect())
                .unwrap_or_else(|| (1..=urls.len()).map(|i| i as u8).collect());

            let provider_names: Vec<String> = env::var("RPC_PROVIDERS")
                .ok()
                .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_else(|| {
                    urls.iter().map(|u| {
                        if u.contains("helius") { "helius".to_string() }
                        else if u.contains("quicknode") { "quicknode".to_string() }
                        else if u.contains("alchemy") { "alchemy".to_string() }
                        else { "public".to_string() }
                    }).collect()
                });

            urls.into_iter().enumerate().map(|(i, url)| RpcEndpointConfig {
                url,
                provider: provider_names.get(i).cloned().unwrap_or_else(|| format!("provider-{}", i + 1)),
                priority: priorities.get(i).copied().unwrap_or((i + 1) as u8),
                ws_url: ws_urls.get(i).and_then(|v| v.clone()),
            }).collect::<Vec<_>>()
        };

        // Critical values must be supplied via environment — fail fast otherwise
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL is required but not set")?;

        // Resolve keypair bytes from WALLET_PRIVATE_KEY (base58 or JSON array) OR KEYPAIR_PATH file
        let keypair_bytes = load_keypair_bytes()?;

        Ok(Self {
            environment: env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
            database_url,
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            grpc_port: env::var("GRPC_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(50051),
            metrics_port: env::var("METRICS_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(9091),
            keypair_bytes,
            rpc_endpoints,
            jito_bundle_url: env::var("JITO_BUNDLE_URL").ok(),
            execution_workers: env::var("EXECUTION_WORKERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            max_concurrent_orders: env::var("MAX_CONCURRENT_ORDERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            order_timeout_seconds: env::var("ORDER_TIMEOUT_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(30),
            risk_limits: RiskLimits {
                max_position_size_sol: env::var("MAX_POSITION_SIZE_SOL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10.0),
                max_portfolio_exposure_sol: env::var("MAX_PORTFOLIO_EXPOSURE_SOL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(100.0),
                max_daily_loss_sol: env::var("MAX_DAILY_LOSS_SOL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5.0),
                max_slippage_bps: env::var("MAX_SLIPPAGE_BPS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(500),
                max_sandwich_risk_score: env::var("MAX_SANDWICH_RISK_SCORE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(70),
            },
            trading: TradingConfig {
                default_slippage_bps: env::var("DEFAULT_SLIPPAGE_BPS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(100),
                min_trade_size_sol: env::var("MIN_TRADE_SIZE_SOL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.01),
                max_trade_size_sol: env::var("MAX_TRADE_SIZE_SOL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10.0),
                mev_protection_enabled: env::var("MEV_PROTECTION_ENABLED")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(true),
                tip_percentage: env::var("TIP_PERCENTAGE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.001),
                retry_attempts: env::var("RETRY_ATTEMPTS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(3),
                retry_delay_ms: env::var("RETRY_DELAY_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1000),
            },
            monitoring: MonitoringConfig {
                log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
                slack_webhook_url: env::var("SLACK_WEBHOOK_URL").ok(),
            },
        })
    }
}

/// Load wallet keypair bytes from:
///   1. WALLET_PRIVATE_KEY env var — accepts a base58-encoded private key string
///      (the 64-byte Solana keypair encoded as base58) OR a JSON byte array
///      like `[1,2,3,...,64]`.
///   2. KEYPAIR_PATH env var — path to a JSON file containing the byte array.
///
/// Returns the raw 64-byte keypair (32 secret bytes + 32 public key bytes).
pub fn load_keypair_bytes() -> Result<Vec<u8>, String> {
    // Try WALLET_PRIVATE_KEY first (no file needed — ideal for Replit secrets)
    if let Ok(key_str) = env::var("WALLET_PRIVATE_KEY") {
        let key_str = key_str.trim();
        // Detect JSON array: starts with '['
        if key_str.starts_with('[') {
            let bytes: Vec<u8> = serde_json::from_str(key_str)
                .map_err(|e| format!("WALLET_PRIVATE_KEY: invalid JSON byte array — {}", e))?;
            if bytes.len() != 64 {
                return Err(format!(
                    "WALLET_PRIVATE_KEY: expected 64 bytes, got {}. \
                     Export your full keypair (secret key + public key).",
                    bytes.len()
                ));
            }
            return Ok(bytes);
        }
        // Otherwise treat as base58
        let bytes = bs58::decode(key_str)
            .into_vec()
            .map_err(|e| format!("WALLET_PRIVATE_KEY: invalid base58 — {}", e))?;
        if bytes.len() != 64 {
            return Err(format!(
                "WALLET_PRIVATE_KEY (base58): expected 64 bytes, got {}.",
                bytes.len()
            ));
        }
        return Ok(bytes);
    }

    // Fall back to KEYPAIR_PATH file
    let path_str = env::var("KEYPAIR_PATH").map_err(|_| {
        "Wallet not configured. Set either:\n  \
         WALLET_PRIVATE_KEY — your base58 or JSON-array private key\n  \
         KEYPAIR_PATH       — path to a Solana keypair JSON file".to_string()
    })?;
    let data = std::fs::read_to_string(&path_str)
        .map_err(|e| format!("KEYPAIR_PATH '{}': cannot read file — {}", path_str, e))?;
    let bytes: Vec<u8> = serde_json::from_str(data.trim())
        .map_err(|e| format!("KEYPAIR_PATH '{}': invalid JSON — {}", path_str, e))?;
    if bytes.len() != 64 {
        return Err(format!(
            "KEYPAIR_PATH '{}': expected 64 bytes, got {}.",
            path_str, bytes.len()
        ));
    }
    Ok(bytes)
}
