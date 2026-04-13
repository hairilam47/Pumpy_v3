use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub environment: String,
    pub database_url: String,
    pub redis_url: String,
    pub grpc_port: u16,
    pub metrics_port: u16,
    pub keypair_path: PathBuf,
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
    pub fn from_env() -> Result<Self, String> {
        let rpc_url = env::var("SOLANA_RPC_URL")
            .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());
        let ws_url = env::var("SOLANA_WS_URL").ok();

        let rpc_endpoints = vec![RpcEndpointConfig {
            url: rpc_url,
            provider: env::var("RPC_PROVIDER").unwrap_or_else(|_| "public".to_string()),
            priority: 1,
            ws_url,
        }];

        Ok(Self {
            environment: env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()),
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgresql://localhost:5432/pumpfun".to_string()),
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
            keypair_path: PathBuf::from(
                env::var("KEYPAIR_PATH").unwrap_or_else(|_| "keypair.json".to_string()),
            ),
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
