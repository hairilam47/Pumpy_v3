pub mod bonding_curve;
pub mod instructions;

use std::sync::Arc;
use std::path::PathBuf;
use std::str::FromStr;
use tokio::sync::broadcast;
use tracing::{info, warn, error};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signer::keypair::Keypair,
    pubkey::Pubkey,
    signer::Signer,
    transaction::Transaction,
    signature::Signature,
};

use crate::rpc::RpcManager;
use crate::constants::*;
use self::instructions::*;
use self::bonding_curve::BondingCurveParams;

/// Event emitted when a new token is discovered
#[derive(Debug, Clone)]
pub struct TokenDiscoveredEvent {
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub uri: String,
    pub creator: String,
    pub bonding_curve: String,
    pub timestamp: i64,
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
}

/// Event emitted when a token's bonding curve updates
#[derive(Debug, Clone)]
pub struct TokenUpdateEvent {
    pub mint: String,
    pub bonding_curve_params: BondingCurveParams,
    pub timestamp: i64,
}

#[derive(Clone)]
pub struct PumpFunClient {
    rpc_manager: Arc<RpcManager>,
    keypair: Arc<Keypair>,
    token_event_tx: broadcast::Sender<TokenDiscoveredEvent>,
}

impl PumpFunClient {
    pub fn new(
        rpc_manager: Arc<RpcManager>,
        keypair_path: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let keypair = load_keypair(&keypair_path)?;
        let (token_event_tx, _) = broadcast::channel(1000);

        info!("PumpFun client initialized with wallet: {}", keypair.pubkey());

        Ok(Self {
            rpc_manager,
            keypair: Arc::new(keypair),
            token_event_tx,
        })
    }

    pub fn subscribe_token_events(&self) -> broadcast::Receiver<TokenDiscoveredEvent> {
        self.token_event_tx.subscribe()
    }

    pub fn pubkey(&self) -> Pubkey {
        self.keypair.pubkey()
    }

    pub async fn get_balance(&self) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        Ok(client.get_balance(&self.keypair.pubkey()).await?)
    }

    pub async fn token_exists(&self, mint: &Pubkey) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        match client.get_account(mint).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub async fn get_bonding_curve_params(
        &self,
        mint: &Pubkey,
    ) -> Result<BondingCurveParams, Box<dyn std::error::Error + Send + Sync>> {
        let (bonding_curve_pda, _) = derive_bonding_curve_pda(mint);
        let client = self.rpc_manager.get_client().await?;
        let account = client.get_account(&bonding_curve_pda).await?;

        // Parse bonding curve account data (simplified)
        let data = &account.data;
        if data.len() < 40 {
            return Ok(BondingCurveParams::default());
        }

        // Skip 8-byte discriminator
        let offset = 8;
        let virtual_token_reserves = u64::from_le_bytes(data[offset..offset + 8].try_into()?);
        let virtual_sol_reserves = u64::from_le_bytes(data[offset + 8..offset + 16].try_into()?);
        let real_token_reserves = u64::from_le_bytes(data[offset + 16..offset + 24].try_into()?);
        let real_sol_reserves = u64::from_le_bytes(data[offset + 24..offset + 32].try_into()?);
        let token_total_supply = u64::from_le_bytes(data[offset + 32..offset + 40].try_into()?);
        let complete = if data.len() > offset + 40 { data[offset + 40] != 0 } else { false };

        Ok(BondingCurveParams {
            virtual_sol_reserves,
            virtual_token_reserves,
            real_sol_reserves,
            real_token_reserves,
            token_total_supply,
            complete,
        })
    }

    pub async fn buy_token(
        &self,
        mint: &Pubkey,
        amount: u64,
        max_sol_cost: u64,
        slippage_bps: u64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        let buyer = self.keypair.pubkey();

        let (bonding_curve, _) = derive_bonding_curve_pda(mint);
        let associated_bonding_curve = get_associated_token_address(&bonding_curve, mint);
        let associated_user = get_associated_token_address(&buyer, mint);

        let ix = build_buy_instruction(
            &buyer,
            mint,
            &bonding_curve,
            &associated_bonding_curve,
            &associated_user,
            amount,
            max_sol_cost,
        );

        let recent_blockhash = client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&buyer),
            &[&*self.keypair],
            recent_blockhash,
        );

        let signature = client
            .send_and_confirm_transaction_with_spinner_and_commitment(
                &tx,
                CommitmentConfig::confirmed(),
            )
            .await?;

        info!("Buy transaction confirmed: {}", signature);
        Ok(signature.to_string())
    }

    pub async fn sell_token(
        &self,
        mint: &Pubkey,
        amount: u64,
        min_sol_output: u64,
        slippage_bps: u64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        let seller = self.keypair.pubkey();

        let (bonding_curve, _) = derive_bonding_curve_pda(mint);
        let associated_bonding_curve = get_associated_token_address(&bonding_curve, mint);
        let associated_user = get_associated_token_address(&seller, mint);

        let ix = build_sell_instruction(
            &seller,
            mint,
            &bonding_curve,
            &associated_bonding_curve,
            &associated_user,
            amount,
            min_sol_output,
        );

        let recent_blockhash = client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&seller),
            &[&*self.keypair],
            recent_blockhash,
        );

        let signature = client
            .send_and_confirm_transaction_with_spinner_and_commitment(
                &tx,
                CommitmentConfig::confirmed(),
            )
            .await?;

        info!("Sell transaction confirmed: {}", signature);
        Ok(signature.to_string())
    }

    /// Publish a discovered token event to all subscribers
    pub fn publish_token_event(&self, event: TokenDiscoveredEvent) -> Result<usize, broadcast::error::SendError<TokenDiscoveredEvent>> {
        self.token_event_tx.send(event)
    }

    /// Build a buy transaction without submitting it (used for Jito bundle submission)
    pub async fn build_buy_transaction(
        &self,
        mint: &Pubkey,
        amount: u64,
        max_sol_cost: u64,
    ) -> Result<(Transaction, solana_sdk::hash::Hash), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        let buyer = self.keypair.pubkey();

        let (bonding_curve, _) = derive_bonding_curve_pda(mint);
        let associated_bonding_curve = get_associated_token_address(&bonding_curve, mint);
        let associated_user = get_associated_token_address(&buyer, mint);

        let ix = build_buy_instruction(
            &buyer,
            mint,
            &bonding_curve,
            &associated_bonding_curve,
            &associated_user,
            amount,
            max_sol_cost,
        );

        let recent_blockhash = client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&buyer),
            &[&*self.keypair],
            recent_blockhash,
        );

        Ok((tx, recent_blockhash))
    }

    /// Build a sell transaction without submitting it (used for Jito bundle submission)
    pub async fn build_sell_transaction(
        &self,
        mint: &Pubkey,
        amount: u64,
        min_sol_output: u64,
    ) -> Result<(Transaction, solana_sdk::hash::Hash), Box<dyn std::error::Error + Send + Sync>> {
        let client = self.rpc_manager.get_client().await?;
        let seller = self.keypair.pubkey();

        let (bonding_curve, _) = derive_bonding_curve_pda(mint);
        let associated_bonding_curve = get_associated_token_address(&bonding_curve, mint);
        let associated_user = get_associated_token_address(&seller, mint);

        let ix = build_sell_instruction(
            &seller,
            mint,
            &bonding_curve,
            &associated_bonding_curve,
            &associated_user,
            amount,
            min_sol_output,
        );

        let recent_blockhash = client.get_latest_blockhash().await?;
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&seller),
            &[&*self.keypair],
            recent_blockhash,
        );

        Ok((tx, recent_blockhash))
    }

    pub async fn start_token_monitor(
        &self,
        _order_manager: Arc<crate::order::OrderManager>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    pub async fn update_positions(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

fn load_keypair(path: &PathBuf) -> Result<Keypair, Box<dyn std::error::Error + Send + Sync>> {
    if path.exists() {
        let data = std::fs::read_to_string(path)?;
        let bytes: Vec<u8> = serde_json::from_str(&data)?;
        Ok(Keypair::from_bytes(&bytes)?)
    } else {
        warn!("Keypair file not found at {:?}, generating ephemeral keypair for testing", path);
        Ok(Keypair::new())
    }
}
