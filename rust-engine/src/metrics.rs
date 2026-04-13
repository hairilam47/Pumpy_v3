use prometheus::{
    Counter, Gauge, Histogram, HistogramOpts, IntCounter, IntGauge,
    Registry, TextEncoder, Encoder,
};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, error};

#[derive(Clone)]
pub struct Metrics {
    registry: Registry,

    // Order metrics
    pub orders_submitted: IntCounter,
    pub orders_executed: IntCounter,
    pub orders_failed: IntCounter,
    pub orders_rejected: IntCounter,
    pub orders_cancelled: IntCounter,
    pub pending_orders: IntGauge,
    pub active_orders: IntGauge,

    // Execution metrics
    pub order_execution_time: Histogram,
    pub transaction_confirmation_time: Histogram,

    // Financial metrics
    pub total_pnl: Gauge,
    pub daily_pnl: Gauge,
    pub wallet_balance: Gauge,
    pub positions_value: Gauge,

    // MEV metrics
    pub jito_bundles_submitted: IntCounter,
    pub jito_bundles_landed: IntCounter,
    pub sandwich_attacks_detected: IntCounter,
    pub mev_saved_sol: Counter,

    // RPC metrics
    pub rpc_requests: IntCounter,
    pub rpc_errors: IntCounter,
    pub rpc_latency: Histogram,

    // Token metrics
    pub tokens_discovered: IntCounter,
    pub tokens_sniped: IntCounter,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let orders_submitted = IntCounter::new("orders_submitted_total", "Total orders submitted")?;
        let orders_executed = IntCounter::new("orders_executed_total", "Total orders executed")?;
        let orders_failed = IntCounter::new("orders_failed_total", "Total orders failed")?;
        let orders_rejected = IntCounter::new("orders_rejected_total", "Total orders rejected")?;
        let orders_cancelled = IntCounter::new("orders_cancelled_total", "Total orders cancelled")?;
        let pending_orders = IntGauge::new("pending_orders", "Current pending orders")?;
        let active_orders = IntGauge::new("active_orders", "Current active orders")?;

        let execution_opts = HistogramOpts::new("order_execution_seconds", "Order execution time")
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]);
        let order_execution_time = Histogram::with_opts(execution_opts)?;

        let confirm_opts = HistogramOpts::new("tx_confirmation_seconds", "Transaction confirmation time")
            .buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]);
        let transaction_confirmation_time = Histogram::with_opts(confirm_opts)?;

        let total_pnl = Gauge::new("total_pnl_sol", "Total PnL in SOL")?;
        let daily_pnl = Gauge::new("daily_pnl_sol", "Daily PnL in SOL")?;
        let wallet_balance = Gauge::new("wallet_balance_sol", "Wallet balance in SOL")?;
        let positions_value = Gauge::new("positions_value_sol", "Open positions value in SOL")?;

        let jito_bundles_submitted =
            IntCounter::new("jito_bundles_submitted_total", "Total Jito bundles submitted")?;
        let jito_bundles_landed =
            IntCounter::new("jito_bundles_landed_total", "Total Jito bundles landed")?;
        let sandwich_attacks_detected =
            IntCounter::new("sandwich_attacks_detected_total", "Sandwich attacks detected")?;
        let mev_saved_sol = Counter::new("mev_saved_sol_total", "Total MEV saved in SOL")?;

        let rpc_requests = IntCounter::new("rpc_requests_total", "Total RPC requests")?;
        let rpc_errors = IntCounter::new("rpc_errors_total", "Total RPC errors")?;
        let rpc_opts = HistogramOpts::new("rpc_latency_seconds", "RPC request latency")
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]);
        let rpc_latency = Histogram::with_opts(rpc_opts)?;

        let tokens_discovered = IntCounter::new("tokens_discovered_total", "Total new tokens discovered")?;
        let tokens_sniped = IntCounter::new("tokens_sniped_total", "Total tokens sniped")?;

        registry.register(Box::new(orders_submitted.clone()))?;
        registry.register(Box::new(orders_executed.clone()))?;
        registry.register(Box::new(orders_failed.clone()))?;
        registry.register(Box::new(orders_rejected.clone()))?;
        registry.register(Box::new(orders_cancelled.clone()))?;
        registry.register(Box::new(pending_orders.clone()))?;
        registry.register(Box::new(active_orders.clone()))?;
        registry.register(Box::new(order_execution_time.clone()))?;
        registry.register(Box::new(transaction_confirmation_time.clone()))?;
        registry.register(Box::new(total_pnl.clone()))?;
        registry.register(Box::new(daily_pnl.clone()))?;
        registry.register(Box::new(wallet_balance.clone()))?;
        registry.register(Box::new(positions_value.clone()))?;
        registry.register(Box::new(jito_bundles_submitted.clone()))?;
        registry.register(Box::new(jito_bundles_landed.clone()))?;
        registry.register(Box::new(sandwich_attacks_detected.clone()))?;
        registry.register(Box::new(mev_saved_sol.clone()))?;
        registry.register(Box::new(rpc_requests.clone()))?;
        registry.register(Box::new(rpc_errors.clone()))?;
        registry.register(Box::new(rpc_latency.clone()))?;
        registry.register(Box::new(tokens_discovered.clone()))?;
        registry.register(Box::new(tokens_sniped.clone()))?;

        Ok(Self {
            registry,
            orders_submitted,
            orders_executed,
            orders_failed,
            orders_rejected,
            orders_cancelled,
            pending_orders,
            active_orders,
            order_execution_time,
            transaction_confirmation_time,
            total_pnl,
            daily_pnl,
            wallet_balance,
            positions_value,
            jito_bundles_submitted,
            jito_bundles_landed,
            sandwich_attacks_detected,
            mev_saved_sol,
            rpc_requests,
            rpc_errors,
            rpc_latency,
            tokens_discovered,
            tokens_sniped,
        })
    }

    pub fn gather_metrics(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut output = Vec::new();
        encoder.encode(&metric_families, &mut output).unwrap_or_default();
        String::from_utf8(output).unwrap_or_default()
    }

    pub async fn start_server(&self, port: u16) {
        let metrics_clone = Arc::new(self.clone());
        let addr = format!("0.0.0.0:{}", port);
        info!("Starting Prometheus metrics server on {}", addr);

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind metrics server: {}", e);
                return;
            }
        };

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let metrics = metrics_clone.clone();
                    tokio::spawn(async move {
                        let body = metrics.gather_metrics();
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        use tokio::io::AsyncWriteExt;
                        let mut stream = stream;
                        let _ = stream.write_all(response.as_bytes()).await;
                    });
                }
                Err(e) => {
                    error!("Metrics server accept error: {}", e);
                }
            }
        }
    }
}
