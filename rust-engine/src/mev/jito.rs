use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_sdk::{
    pubkey::Pubkey,
    transaction::Transaction,
};
use std::time::Duration;
use tracing::info;
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
    /// Optional backup RPC endpoint used for pre-submission simulation.
    /// When `None`, simulation is skipped entirely.
    sim_rpc_url: Option<String>,
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
            sim_rpc_url: None,
        }
    }

    /// Attach a backup RPC endpoint for pre-submission simulation.
    /// Calls to `simulate_transaction` will use this URL; without it, simulation is skipped.
    pub fn with_sim_rpc(mut self, url: String) -> Self {
        self.sim_rpc_url = Some(url);
        self
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

    /// Compute a dynamic Jito tip from trade value and configurable parameters.
    ///
    /// Formula: `tip = clamp(trade_value_lamports * tip_percent, floor, ceiling)`
    ///
    /// All three parameters come from `bot_config` at order time so operators can
    /// tune MEV aggressiveness without restarting the engine.
    pub fn compute_dynamic_tip(
        trade_value_lamports: u64,
        tip_percent: f64,
        floor_lamports: u64,
        ceiling_lamports: u64,
    ) -> u64 {
        let raw = (trade_value_lamports as f64 * tip_percent) as u64;
        raw.max(floor_lamports).min(ceiling_lamports)
    }

    /// Run `simulateTransaction` against the configured backup RPC.
    ///
    /// Returns `Ok(())` if simulation succeeds or if no sim RPC is configured.
    /// Returns `Err(reason)` if the simulation returns an error, letting the caller
    /// reject the order before it consumes a Jito bundle slot.
    pub async fn simulate_transaction(&self, tx: &Transaction) -> Result<(), String> {
        let sim_url = match &self.sim_rpc_url {
            Some(url) => url.clone(),
            None => {
                info!("No sim RPC configured — skipping pre-submission simulation");
                return Ok(());
            }
        };

        let bytes = bincode::serialize(tx)
            .map_err(|e| format!("tx serialization error: {}", e))?;
        let encoded = base64::encode(&bytes);

        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [
                encoded,
                {
                    "encoding": "base64",
                    "commitment": "confirmed",
                    "replaceRecentBlockhash": true
                }
            ]
        });

        let response = self
            .client
            .post(&sim_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("simulation request failed: {}", e))?;

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("simulation response parse error: {}", e))?;

        // RPC-level error (wrong method, auth failure, etc.)
        if let Some(rpc_err) = result.get("error") {
            return Err(format!("simulation RPC error: {}", rpc_err));
        }

        // Transaction-level simulation error
        let sim_err = &result["result"]["value"]["err"];
        if !sim_err.is_null() {
            return Err(format!("simulation failed: {}", sim_err));
        }

        info!("Pre-submission simulation passed");
        Ok(())
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

    /// Parse the raw `JITO_SIMULATION_ENABLED` bot_config string into a boolean.
    ///
    /// The function is extracted here so it can be unit-tested without a database.
    /// Callers in `order::manager` pass the raw `Option<String>` returned by
    /// `database::get_config_value`.
    ///
    /// Returns `true` (simulation enabled) unless the value is explicitly `"false"` or `"0"`.
    pub fn sim_enabled_from_str(val: Option<String>) -> bool {
        val.map(|v| v != "false" && v != "0").unwrap_or(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{hash::Hash, signature::Keypair, signer::Signer, system_instruction};

    // ─── helpers ──────────────────────────────────────────────────────────────

    /// Build a minimal signed transaction that serialises without error.
    /// The blockhash is `Hash::default()` (all-zeros) which is invalid on-chain
    /// but perfectly fine for testing serialisation and HTTP round-trips.
    fn dummy_signed_tx() -> solana_sdk::transaction::Transaction {
        let payer = Keypair::new();
        let ix = system_instruction::transfer(&payer.pubkey(), &payer.pubkey(), 0);
        solana_sdk::transaction::Transaction::new_signed_with_payer(
            &[ix],
            Some(&payer.pubkey()),
            &[&payer],
            Hash::default(),
        )
    }

    /// Spawn a one-shot HTTP/1.1 server on an OS-assigned port.
    ///
    /// The server accepts **one** connection, discards the request body, and
    /// writes `response_json` as the HTTP response body before closing. This
    /// is sufficient for `reqwest` to parse the JSON without a keep-alive loop.
    async fn spawn_mock_rpc(response_json: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = stream.read(&mut buf).await;
                let http = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_json.len(),
                    response_json,
                );
                let _ = stream.write_all(http.as_bytes()).await;
            }
        });
        format!("http://{}", addr)
    }

    // ─── Case 1 ───────────────────────────────────────────────────────────────
    // No sim RPC URL configured → simulate_transaction returns Ok(()) immediately
    // without making any network request.  This is the behaviour when the engine
    // is run with a single RPC (no dedicated simulation node).

    #[tokio::test]
    async fn test_sim_no_rpc_url_returns_ok() {
        let jito = JitoClient::new("http://127.0.0.1:1/unreachable".to_string());
        // No .with_sim_rpc() call → sim_rpc_url is None
        let result = jito.simulate_transaction(&dummy_signed_tx()).await;
        assert!(
            result.is_ok(),
            "Expected Ok(()) when no sim RPC URL is configured, got: {:?}",
            result.err()
        );
    }

    // ─── Case 2 ───────────────────────────────────────────────────────────────
    // Sim RPC returns a transaction-level error → simulate_transaction returns
    // Err whose message starts with "simulation failed:", matching the prefix
    // the caller uses to set the simulation_rejected: order error.

    #[tokio::test]
    async fn test_sim_rpc_tx_error_is_rejected_with_correct_prefix() {
        // RPC response that carries a transaction-level simulation error.
        const ERR_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"value":{"err":{"InstructionError":[0,"InvalidAccountData"]},"logs":[]}}}"#;
        let url = spawn_mock_rpc(ERR_BODY).await;

        let jito = JitoClient::new("http://127.0.0.1:1/unreachable".to_string())
            .with_sim_rpc(url);

        let result = jito.simulate_transaction(&dummy_signed_tx()).await;
        assert!(result.is_err(), "Expected Err when simulation returns a tx error");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("simulation failed:"),
            "Error prefix mismatch — got: {msg}"
        );
    }

    // ─── Case 3 ───────────────────────────────────────────────────────────────
    // Sim RPC returns a successful simulation (null err) → simulate_transaction
    // returns Ok(()) and execution proceeds.

    #[tokio::test]
    async fn test_sim_rpc_success_returns_ok() {
        const OK_BODY: &str =
            r#"{"jsonrpc":"2.0","id":1,"result":{"value":{"err":null,"logs":[]}}}"#;
        let url = spawn_mock_rpc(OK_BODY).await;

        let jito = JitoClient::new("http://127.0.0.1:1/unreachable".to_string())
            .with_sim_rpc(url);

        let result = jito.simulate_transaction(&dummy_signed_tx()).await;
        assert!(
            result.is_ok(),
            "Expected Ok(()) when simulation passes, got: {:?}",
            result.err()
        );
    }

    // ─── Case 4 ───────────────────────────────────────────────────────────────
    // sim_enabled_from_str covers the JITO_SIMULATION_ENABLED=false bypass path.
    // In the manager, when this returns false the sim block is skipped entirely
    // and simulate_transaction is never called (so a broken sim RPC is harmless).

    #[test]
    fn test_sim_enabled_from_str_covers_all_cases() {
        // Absent key → enabled (safe default: always simulate when configured)
        assert!(JitoClient::sim_enabled_from_str(None), "None should default to enabled");
        // Truthy strings → enabled
        assert!(JitoClient::sim_enabled_from_str(Some("true".into())));
        assert!(JitoClient::sim_enabled_from_str(Some("1".into())));
        // Explicit opt-out values → disabled (JITO_SIMULATION_ENABLED=false bypasses sim)
        assert!(!JitoClient::sim_enabled_from_str(Some("false".into())));
        assert!(!JitoClient::sim_enabled_from_str(Some("0".into())));
        // Empty or unrecognised → treated as enabled (conservative)
        assert!(JitoClient::sim_enabled_from_str(Some("".into())));
        assert!(JitoClient::sim_enabled_from_str(Some("yes".into())));
    }

    // ─── Case 5 ───────────────────────────────────────────────────────────────
    // Sim RPC returns an RPC-level error (wrong method, auth failure) →
    // simulate_transaction returns Err with "simulation RPC error:" prefix.

    #[tokio::test]
    async fn test_sim_rpc_level_error_is_rejected() {
        const RPC_ERR_BODY: &str =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let url = spawn_mock_rpc(RPC_ERR_BODY).await;

        let jito = JitoClient::new("http://127.0.0.1:1/unreachable".to_string())
            .with_sim_rpc(url);

        let result = jito.simulate_transaction(&dummy_signed_tx()).await;
        assert!(result.is_err(), "Expected Err when RPC returns an error object");
        let msg = result.unwrap_err();
        assert!(
            msg.starts_with("simulation RPC error:"),
            "Error prefix mismatch — got: {msg}"
        );
    }
}
