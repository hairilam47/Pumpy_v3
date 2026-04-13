use crate::constants::*;

#[derive(Debug, Clone, Copy)]
pub struct BondingCurveParams {
    pub virtual_sol_reserves: u64,
    pub virtual_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
}

impl Default for BondingCurveParams {
    fn default() -> Self {
        Self {
            virtual_sol_reserves: BONDING_CURVE_INITIAL_VIRTUAL_SOL_RESERVES,
            virtual_token_reserves: BONDING_CURVE_INITIAL_VIRTUAL_TOKEN_RESERVES,
            real_sol_reserves: 0,
            real_token_reserves: BONDING_CURVE_INITIAL_REAL_TOKEN_RESERVES,
            token_total_supply: BONDING_CURVE_TOKEN_TOTAL_SUPPLY,
            complete: false,
        }
    }
}

impl BondingCurveParams {
    /// Calculate tokens received for a given SOL input (buy)
    pub fn tokens_for_sol(&self, sol_amount: u64) -> u64 {
        if sol_amount == 0 || self.complete {
            return 0;
        }
        let product = (self.virtual_sol_reserves as u128)
            .saturating_mul(self.virtual_token_reserves as u128);
        let new_sol_reserves = (self.virtual_sol_reserves as u128).saturating_add(sol_amount as u128);
        let new_token_reserves = product / new_sol_reserves;
        let tokens_out = (self.virtual_token_reserves as u128).saturating_sub(new_token_reserves);
        tokens_out.min(self.real_token_reserves as u128) as u64
    }

    /// Calculate SOL received for a given token input (sell)
    pub fn sol_for_tokens(&self, token_amount: u64) -> u64 {
        if token_amount == 0 || self.complete {
            return 0;
        }
        let product = (self.virtual_sol_reserves as u128)
            .saturating_mul(self.virtual_token_reserves as u128);
        let new_token_reserves =
            (self.virtual_token_reserves as u128).saturating_add(token_amount as u128);
        let new_sol_reserves = product / new_token_reserves;
        let sol_out = (self.virtual_sol_reserves as u128).saturating_sub(new_sol_reserves);
        sol_out.min(self.real_sol_reserves as u128) as u64
    }

    /// Get current token price in SOL (per token, in lamports/token)
    pub fn token_price_lamports(&self) -> f64 {
        if self.virtual_token_reserves == 0 {
            return 0.0;
        }
        self.virtual_sol_reserves as f64 / self.virtual_token_reserves as f64
    }

    /// Get market cap in SOL
    pub fn market_cap_sol(&self) -> f64 {
        let price = self.token_price_lamports();
        price * self.token_total_supply as f64 / LAMPORTS_PER_SOL as f64
    }

    /// Get bonding curve completion percentage (0-100)
    pub fn bonding_curve_progress(&self) -> f64 {
        let total_real = BONDING_CURVE_INITIAL_REAL_TOKEN_RESERVES as f64;
        if total_real == 0.0 {
            return 0.0;
        }
        let sold = total_real - self.real_token_reserves as f64;
        (sold / total_real * 100.0).clamp(0.0, 100.0)
    }

    /// Apply slippage to calculate max cost for a buy
    pub fn max_sol_cost_with_slippage(&self, sol_amount: u64, slippage_bps: u64) -> u64 {
        sol_amount + (sol_amount * slippage_bps / 10_000)
    }

    /// Apply slippage to calculate min output for a sell
    pub fn min_sol_output_with_slippage(&self, sol_amount: u64, slippage_bps: u64) -> u64 {
        sol_amount.saturating_sub(sol_amount * slippage_bps / 10_000)
    }
}
