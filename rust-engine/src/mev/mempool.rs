use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, debug};

#[derive(Debug, Clone)]
pub struct TransactionInfo {
    pub signature: String,
    pub accounts: Vec<String>,
    pub program_ids: Vec<String>,
    pub timestamp: Instant,
}

pub struct MempoolMonitor {
    pending_transactions: Arc<RwLock<VecDeque<TransactionInfo>>>,
    max_cache_size: usize,
}

impl MempoolMonitor {
    pub fn new(max_cache_size: usize) -> Self {
        Self {
            pending_transactions: Arc::new(RwLock::new(VecDeque::new())),
            max_cache_size,
        }
    }

    pub async fn add_transaction(&self, tx: TransactionInfo) {
        let mut cache = self.pending_transactions.write().await;
        if cache.len() >= self.max_cache_size {
            cache.pop_front();
        }
        cache.push_back(tx);
    }

    pub async fn get_recent_transactions(&self, max_age: Duration) -> Vec<TransactionInfo> {
        let cache = self.pending_transactions.read().await;
        let now = Instant::now();
        cache
            .iter()
            .filter(|tx| now.duration_since(tx.timestamp) <= max_age)
            .cloned()
            .collect()
    }

    pub async fn find_targeting_accounts(&self, accounts: &[String]) -> Vec<TransactionInfo> {
        let cache = self.pending_transactions.read().await;
        let account_set: std::collections::HashSet<&str> =
            accounts.iter().map(|s| s.as_str()).collect();
        cache
            .iter()
            .filter(|tx| tx.accounts.iter().any(|a| account_set.contains(a.as_str())))
            .cloned()
            .collect()
    }

    pub async fn prune_old_entries(&self, max_age: Duration) {
        let mut cache = self.pending_transactions.write().await;
        let now = Instant::now();
        cache.retain(|tx| now.duration_since(tx.timestamp) <= max_age);
    }
}
