use solana_sdk::pubkey;
use solana_sdk::pubkey::Pubkey;

pub const PUMPFUN_PROGRAM_ID: Pubkey =
    pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

pub const PUMPFUN_DEVNET_PROGRAM_ID: Pubkey =
    pubkey!("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P");

pub const GLOBAL_ACCOUNT_SEED: &[u8] = b"global";
pub const BONDING_CURVE_SEED: &[u8] = b"bonding-curve";
pub const ASSOCIATED_BONDING_CURVE_SEED: &[u8] = b"associated-bonding-curve";
pub const METADATA_SEED: &[u8] = b"metadata";

pub const FEE_RECIPIENT: Pubkey =
    pubkey!("CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM");

pub const LAMPORTS_PER_SOL: u64 = 1_000_000_000;

pub const DEFAULT_SLIPPAGE_BPS: u64 = 100;
pub const MAX_SLIPPAGE_BPS: u64 = 1_000;

pub const BONDING_CURVE_INITIAL_VIRTUAL_TOKEN_RESERVES: u64 = 1_073_000_000_000_000;
pub const BONDING_CURVE_INITIAL_VIRTUAL_SOL_RESERVES: u64 = 30_000_000_000;
pub const BONDING_CURVE_INITIAL_REAL_TOKEN_RESERVES: u64 = 793_100_000_000_000;
pub const BONDING_CURVE_TOKEN_TOTAL_SUPPLY: u64 = 1_000_000_000_000_000;
