use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    signer::keypair::Keypair,
    pubkey::Pubkey,
    signer::Signer,
    transaction::Transaction,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn, error};
use rand::seq::SliceRandom;

const JITO_TIP_ACCOUNTS: &[&str] = &[
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

#[derive(Debug, Serialize, Deserialize)]
pub struct JitoBundle {
    pub transactions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JitoBundleResult {
    pub bundle_id: String,
    pub status: String,
}

#[derive(Clone)]
pub struct JitoClient {
    client: Client,
    bundle_url: String,
    tip_accounts: Vec<Pubkey>,
}

impl JitoClient {
    pub fn new(bundle_url: String) -> Self {
        let tip_accounts: Vec<Pubkey> = JITO_TIP_ACCOUNTS
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect();

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            bundle_url,
            tip_accounts,
        }
    }

    pub fn get_tip_account(&self) -> Option<&Pubkey> {
        let mut rng = rand::thread_rng();
        self.tip_accounts.choose(&mut rng)
    }

    pub fn create_tip_instruction(
        &self,
        payer: &Pubkey,
        tip_lamports: u64,
    ) -> Option<solana_sdk::instruction::Instruction> {
        let tip_account = self.get_tip_account()?;
        Some(solana_sdk::system_instruction::transfer(payer, tip_account, tip_lamports))
    }

    pub async fn send_bundle(
        &self,
        transactions: Vec<Transaction>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let encoded: Vec<String> = transactions
            .iter()
            .map(|tx| {
                let bytes = bincode::serialize(tx).unwrap_or_default();
                base64::encode(&bytes)
            })
            .collect();

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded]
        });

        let response = self
            .client
            .post(&self.bundle_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let result: serde_json::Value = response.json().await?;

        if let Some(bundle_id) = result["result"].as_str() {
            info!("Jito bundle submitted: {}", bundle_id);
            Ok(bundle_id.to_string())
        } else if let Some(error) = result.get("error") {
            Err(format!("Jito bundle error: {}", error).into())
        } else {
            Err("Unknown Jito response".into())
        }
    }

    pub async fn get_bundle_status(
        &self,
        bundle_id: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });

        let response = self
            .client
            .post(&self.bundle_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let result: serde_json::Value = response.json().await?;

        if let Some(contexts) = result["result"]["value"].as_array() {
            if let Some(ctx) = contexts.first() {
                if let Some(status) = ctx["confirmation_status"].as_str() {
                    return Ok(status.to_string());
                }
            }
        }

        Ok("unknown".to_string())
    }
}
