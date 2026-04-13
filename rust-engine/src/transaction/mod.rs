use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    signer::keypair::Keypair,
    signer::Signer,
    transaction::Transaction,
};

/// Build a signed transaction from a list of instructions
pub fn build_transaction(
    instructions: &[Instruction],
    payer: &Keypair,
    recent_blockhash: Hash,
) -> Transaction {
    Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &[payer],
        recent_blockhash,
    )
}

/// Estimate compute units for a set of instructions
pub fn estimate_compute_units(instructions: &[Instruction]) -> u32 {
    // Conservative estimate: 100k CUs per instruction
    instructions.len() as u32 * 100_000
}

/// Add priority fee instruction
pub fn add_priority_fee_instruction(
    instructions: &mut Vec<Instruction>,
    micro_lamports_per_cu: u64,
) {
    let cu_limit = solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_limit(
        200_000,
    );
    let cu_price = solana_sdk::compute_budget::ComputeBudgetInstruction::set_compute_unit_price(
        micro_lamports_per_cu,
    );
    instructions.insert(0, cu_price);
    instructions.insert(0, cu_limit);
}
