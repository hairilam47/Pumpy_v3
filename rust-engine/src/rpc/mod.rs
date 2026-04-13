use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use rand::seq::SliceRandom;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::commitment_config::CommitmentConfig;
use tracing::{info, warn, error};

use crate::config::RpcEndpointConfig;
use crate::metrics::Metrics;

#[derive(Debug, Clone)]
pub struct RpcEndpoint {
    pub config: RpcEndpointConfig,
    pub is_healthy: bool,
    pub latency_ms: u64,
    pub error_count: u32,
    pub last_check: Option<Instant>,
}

#[derive(Clone)]
pub struct RpcManager {
    endpoints: Arc<RwLock<Vec<RpcEndpoint>>>,
    metrics: Option<Arc<Metrics>>,
}

impl RpcManager {
    pub async fn new(configs: Vec<RpcEndpointConfig>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let endpoints: Vec<RpcEndpoint> = configs
            .into_iter()
            .map(|c| RpcEndpoint {
                config: c,
                is_healthy: true,
                latency_ms: 0,
                error_count: 0,
                last_check: None,
            })
            .collect();

        info!("RPC manager initialized with {} endpoints", endpoints.len());

        Ok(Self {
            endpoints: Arc::new(RwLock::new(endpoints)),
            metrics: None,
        })
    }

    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    pub fn start_health_checks(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                manager.check_all_endpoints().await;
            }
        });
    }

    async fn check_all_endpoints(&self) {
        let mut endpoints = self.endpoints.write().await;
        for endpoint in endpoints.iter_mut() {
            let start = Instant::now();
            let client = RpcClient::new_with_commitment(
                endpoint.config.url.clone(),
                CommitmentConfig::confirmed(),
            );
            match client.get_slot().await {
                Ok(_) => {
                    endpoint.is_healthy = true;
                    endpoint.latency_ms = start.elapsed().as_millis() as u64;
                    endpoint.error_count = 0;
                    endpoint.last_check = Some(Instant::now());
                }
                Err(e) => {
                    endpoint.is_healthy = false;
                    endpoint.error_count += 1;
                    endpoint.last_check = Some(Instant::now());
                    warn!("RPC endpoint {} unhealthy: {}", endpoint.config.url, e);
                }
            }
        }
    }

    pub async fn get_client(&self) -> Result<RpcClient, String> {
        let endpoints = self.endpoints.read().await;
        let healthy: Vec<&RpcEndpoint> = endpoints.iter().filter(|e| e.is_healthy).collect();

        if healthy.is_empty() {
            // Fallback: use any endpoint (even unhealthy) rather than failing
            if let Some(ep) = endpoints.first() {
                warn!("No healthy RPC endpoints; falling back to {}", ep.config.url);
                return Ok(RpcClient::new_with_commitment(
                    ep.config.url.clone(),
                    CommitmentConfig::confirmed(),
                ));
            }
            return Err("No RPC endpoints configured".to_string());
        }

        // Priority-based selection: find the highest priority (lowest number = highest priority).
        // Among endpoints sharing the highest priority tier, choose the one with the lowest latency.
        let best_priority = healthy.iter().map(|e| e.config.priority).min().unwrap_or(255);
        let top_tier: Vec<&RpcEndpoint> = healthy
            .iter()
            .filter(|e| e.config.priority == best_priority)
            .copied()
            .collect();

        // Within the top-priority tier, pick the fastest (lowest latency_ms).
        // On equal latency, add lightweight jitter to distribute load.
        let selected = top_tier
            .iter()
            .min_by_key(|e| e.latency_ms)
            .copied()
            .unwrap_or(top_tier[0]);

        if let Some(metrics) = &self.metrics {
            metrics.rpc_requests.inc();
        }

        Ok(RpcClient::new_with_commitment(
            selected.config.url.clone(),
            CommitmentConfig::confirmed(),
        ))
    }

    pub async fn get_websocket_url(&self) -> Option<String> {
        let endpoints = self.endpoints.read().await;
        for ep in endpoints.iter() {
            if let Some(ws) = &ep.config.ws_url {
                return Some(ws.clone());
            }
        }
        // Derive ws URL from http URL
        if let Some(ep) = endpoints.first() {
            let url = ep.config.url.replace("https://", "wss://").replace("http://", "ws://");
            return Some(url);
        }
        None
    }

    pub async fn mark_error(&self, url: &str) {
        let mut endpoints = self.endpoints.write().await;
        for ep in endpoints.iter_mut() {
            if ep.config.url == url {
                ep.error_count += 1;
                if ep.error_count > 5 {
                    ep.is_healthy = false;
                    error!("RPC endpoint {} marked unhealthy after {} errors", url, ep.error_count);
                }
                break;
            }
        }
    }
}
