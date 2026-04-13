use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};
use crate::constants::PUMPFUN_PROGRAM_ID;

const BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
const SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
const CREATE_DISCRIMINATOR: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];

/// Build buy instruction for Pump.fun bonding curve
pub fn build_buy_instruction(
    buyer: &Pubkey,
    mint: &Pubkey,
    bonding_curve: &Pubkey,
    associated_bonding_curve: &Pubkey,
    associated_user: &Pubkey,
    amount: u64,
    max_sol_cost: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&BUY_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&max_sol_cost.to_le_bytes());

    let global = derive_global_pda();
    let fee_recipient = crate::constants::FEE_RECIPIENT;
    let event_authority = derive_event_authority_pda();

    Instruction {
        program_id: PUMPFUN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(global, false),
            AccountMeta::new(fee_recipient, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*bonding_curve, false),
            AccountMeta::new(*associated_bonding_curve, false),
            AccountMeta::new(*associated_user, false),
            AccountMeta::new(*buyer, true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token_id(), false),
            AccountMeta::new_readonly(spl_associated_token_account_id(), false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(PUMPFUN_PROGRAM_ID, false),
        ],
        data,
    }
}

/// Build sell instruction for Pump.fun bonding curve
pub fn build_sell_instruction(
    seller: &Pubkey,
    mint: &Pubkey,
    bonding_curve: &Pubkey,
    associated_bonding_curve: &Pubkey,
    associated_user: &Pubkey,
    amount: u64,
    min_sol_output: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&SELL_DISCRIMINATOR);
    data.extend_from_slice(&amount.to_le_bytes());
    data.extend_from_slice(&min_sol_output.to_le_bytes());

    let global = derive_global_pda();
    let fee_recipient = crate::constants::FEE_RECIPIENT;
    let event_authority = derive_event_authority_pda();

    Instruction {
        program_id: PUMPFUN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new_readonly(global, false),
            AccountMeta::new(fee_recipient, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new(*bonding_curve, false),
            AccountMeta::new(*associated_bonding_curve, false),
            AccountMeta::new(*associated_user, false),
            AccountMeta::new(*seller, true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_associated_token_account_id(), false),
            AccountMeta::new_readonly(spl_token_id(), false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(PUMPFUN_PROGRAM_ID, false),
        ],
        data,
    }
}

pub fn derive_bonding_curve_pda(mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[crate::constants::BONDING_CURVE_SEED, mint.as_ref()],
        &PUMPFUN_PROGRAM_ID,
    )
}

pub fn derive_global_pda() -> Pubkey {
    Pubkey::find_program_address(&[crate::constants::GLOBAL_ACCOUNT_SEED], &PUMPFUN_PROGRAM_ID).0
}

pub fn derive_event_authority_pda() -> Pubkey {
    Pubkey::find_program_address(&[b"__event_authority"], &PUMPFUN_PROGRAM_ID).0
}

fn spl_token_id() -> Pubkey {
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        .parse()
        .unwrap()
}

fn spl_associated_token_account_id() -> Pubkey {
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJe1bJ"
        .parse()
        .unwrap()
}

pub fn get_associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let token_program = spl_token_id();
    let ata_program = spl_associated_token_account_id();
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ata_program,
    )
    .0
}
