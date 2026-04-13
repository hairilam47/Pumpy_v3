pub mod jito;
pub mod mempool;
pub mod sandwich;

use std::sync::Arc;
use tracing::{info, warn};

use crate::metrics::Metrics;
use crate::pumpfun::PumpFunClient;
use self::jito::JitoClient;
use self::mempool::MempoolMonitor;
use self::sandwich::{SandwichDetector, SandwichRiskAnalysis};

#[derive(Clone)]
pub struct MevProtector {
    jito_client: Option<JitoClient>,
    mempool_monitor: Arc<MempoolMonitor>,
    sandwich_detector: SandwichDetector,
    metrics: Arc<Metrics>,
    enabled: bool,
}

impl MevProtector {
    pub fn new(
        jito_bundle_url: Option<String>,
        _pumpfun_client: Arc<PumpFunClient>,
        metrics: Arc<Metrics>,
        max_sandwich_risk: u32,
        enabled: bool,
    ) -> Self {
        let jito_client = jito_bundle_url.map(JitoClient::new);
        let mempool_monitor = Arc::new(MempoolMonitor::new(10_000));
        let sandwich_detector = SandwichDetector::new(mempool_monitor.clone(), max_sandwich_risk);

        Self {
            jito_client,
            mempool_monitor,
            sandwich_detector,
            metrics,
            enabled,
        }
    }

    pub async fn analyze_sandwich_risk(
        &self,
        mint: &str,
        accounts: &[String],
    ) -> SandwichRiskAnalysis {
        if !self.enabled {
            return SandwichRiskAnalysis {
                score: 0,
                risk_level: sandwich::RiskLevel::Low,
                suspicious_txs: vec![],
                reason: "MEV protection disabled".to_string(),
            };
        }
        let analysis = self.sandwich_detector.analyze_risk(mint, accounts).await;
        if analysis.score > 50 {
            self.metrics.sandwich_attacks_detected.inc();
        }
        analysis
    }

    pub fn has_jito(&self) -> bool {
        self.jito_client.is_some()
    }

    pub async fn submit_jito_bundle(
        &self,
        transactions: Vec<solana_sdk::transaction::Transaction>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(jito) = &self.jito_client {
            self.metrics.jito_bundles_submitted.inc();
            let result = jito.send_bundle(transactions).await?;
            self.metrics.jito_bundles_landed.inc();
            Ok(result)
        } else {
            Err("Jito client not configured".into())
        }
    }

    pub fn get_jito_tip_instruction(
        &self,
        payer: &solana_sdk::pubkey::Pubkey,
        tip_lamports: u64,
    ) -> Option<solana_sdk::instruction::Instruction> {
        self.jito_client.as_ref()?.create_tip_instruction(payer, tip_lamports)
    }

    pub fn mempool_monitor(&self) -> &Arc<MempoolMonitor> {
        &self.mempool_monitor
    }
}
