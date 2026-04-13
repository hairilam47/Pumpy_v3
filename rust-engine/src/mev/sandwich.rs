use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::debug;

use crate::mev::mempool::{MempoolMonitor, TransactionInfo};

#[derive(Debug, Clone)]
pub struct SandwichRiskAnalysis {
    pub score: u32,
    pub risk_level: RiskLevel,
    pub suspicious_txs: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone)]
pub struct SandwichDetector {
    mempool_monitor: Arc<MempoolMonitor>,
    analysis_cache: Arc<RwLock<HashMap<String, CachedAnalysis>>>,
    cache_ttl: Duration,
    risk_threshold: u32,
}

#[derive(Clone)]
struct CachedAnalysis {
    analysis: SandwichRiskAnalysis,
    cached_at: Instant,
}

impl SandwichDetector {
    pub fn new(mempool_monitor: Arc<MempoolMonitor>, risk_threshold: u32) -> Self {
        Self {
            mempool_monitor,
            analysis_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::from_secs(5),
            risk_threshold,
        }
    }

    pub async fn analyze_risk(&self, mint: &str, accounts: &[String]) -> SandwichRiskAnalysis {
        // Check cache first
        {
            let cache = self.analysis_cache.read().await;
            if let Some(cached) = cache.get(mint) {
                if Instant::now().duration_since(cached.cached_at) < self.cache_ttl {
                    return cached.analysis.clone();
                }
            }
        }

        let recent_txs = self
            .mempool_monitor
            .get_recent_transactions(Duration::from_secs(2))
            .await;

        let targeting = self
            .mempool_monitor
            .find_targeting_accounts(accounts)
            .await;

        let mut score = 0u32;
        let mut suspicious_txs = Vec::new();
        let mut reasons = Vec::new();

        // Check for multiple transactions targeting same accounts
        if targeting.len() > 2 {
            score += 30;
            reasons.push(format!("{} pending txs targeting same accounts", targeting.len()));
            for tx in &targeting {
                suspicious_txs.push(tx.signature.clone());
            }
        }

        // Check for high-frequency targeting (sandwich indicator)
        if targeting.len() > 5 {
            score += 40;
            reasons.push("High frequency targeting detected".to_string());
        }

        let risk_level = match score {
            0..=25 => RiskLevel::Low,
            26..=50 => RiskLevel::Medium,
            51..=75 => RiskLevel::High,
            _ => RiskLevel::Critical,
        };

        let analysis = SandwichRiskAnalysis {
            score,
            risk_level,
            suspicious_txs,
            reason: reasons.join("; "),
        };

        // Cache the result
        {
            let mut cache = self.analysis_cache.write().await;
            cache.insert(
                mint.to_string(),
                CachedAnalysis {
                    analysis: analysis.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        analysis
    }

    pub async fn prune_cache(&self) {
        let mut cache = self.analysis_cache.write().await;
        let now = Instant::now();
        cache.retain(|_, v| now.duration_since(v.cached_at) < self.cache_ttl * 10);
    }
}
