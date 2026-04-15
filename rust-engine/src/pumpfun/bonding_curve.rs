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

    /// Compute price impact for a buy (SOL → tokens) based on pool depth.
    /// Returns `(tokens_out, price_impact_bps, max_sol_cost)`.
    /// `max_slippage_bps` is the configured ceiling; the actual buffer applied is
    /// `clamp(price_impact_bps * 1.5, 50, max_slippage_bps)` so thin pools
    /// automatically widen the tolerance while deep pools keep it tight.
    pub fn compute_buy_params(&self, sol_amount: u64, max_slippage_bps: u64) -> (u64, u64, u64) {
        let tokens_out = self.tokens_for_sol(sol_amount);

        let price_impact_bps = if self.virtual_sol_reserves > 0 && sol_amount > 0 {
            let impact = (sol_amount as u128 * 10_000)
                / (self.virtual_sol_reserves as u128 + sol_amount as u128);
            impact as u64
        } else {
            max_slippage_bps
        };

        let applied_bps = ((price_impact_bps * 3 / 2).max(50)).min(max_slippage_bps);
        let max_sol_cost = sol_amount + sol_amount * applied_bps / 10_000;
        (tokens_out, price_impact_bps, max_sol_cost)
    }

    /// Compute price impact for a sell (tokens → SOL) based on pool depth.
    /// Returns `(sol_out, price_impact_bps, min_sol_output)`.
    pub fn compute_sell_params(&self, token_amount: u64, max_slippage_bps: u64) -> (u64, u64, u64) {
        let sol_out = self.sol_for_tokens(token_amount);

        let price_impact_bps = if self.virtual_token_reserves > 0 && token_amount > 0 {
            let impact = (token_amount as u128 * 10_000)
                / (self.virtual_token_reserves as u128 + token_amount as u128);
            impact as u64
        } else {
            max_slippage_bps
        };

        let applied_bps = ((price_impact_bps * 3 / 2).max(50)).min(max_slippage_bps);
        let min_sol_output = sol_out.saturating_sub(sol_out * applied_bps / 10_000);
        (sol_out, price_impact_bps, min_sol_output)
    }

    /// Compute the SOL cost required to buy exactly `token_amount` tokens,
    /// using the constant-product formula (ceiling division to ensure the
    /// on-chain instruction never undershoots the required payment).
    ///
    /// Returns `u64::MAX` when the pool cannot satisfy the request (e.g. the
    /// pool is exhausted or `token_amount >= virtual_token_reserves`).
    pub fn sol_cost_for_tokens(&self, token_amount: u64) -> u64 {
        if token_amount == 0 || self.complete {
            return 0;
        }
        let v_tok = self.virtual_token_reserves as u128;
        let v_sol = self.virtual_sol_reserves as u128;
        let t = token_amount as u128;
        if t >= v_tok {
            return u64::MAX;
        }
        // Constant product: sol_in = V_sol * t / (V_tok - t)   [ceiling]
        let numerator = v_sol.saturating_mul(t);
        let denominator = v_tok - t;
        let sol_in = (numerator + denominator - 1) / denominator;
        sol_in.min(u64::MAX as u128) as u64
    }

    /// Compute dynamic slippage bounds for both buy and sell sides based on
    /// the current pool depth and the requested slippage tolerance.
    ///
    /// For a **buy** of `token_amount` tokens:
    ///   `max_sol_cost` = exact SOL cost (price-impact included) + slippage buffer
    ///
    /// For a **sell** of `token_amount` tokens:
    ///   `min_sol_output` = expected SOL output (price-impact included) − slippage buffer
    ///
    /// Returns `(max_sol_cost, min_sol_output)`.
    pub fn calculate_price_impact(&self, token_amount: u64, slippage_bps: u64) -> (u64, u64) {
        let expected_sol_cost = self.sol_cost_for_tokens(token_amount);
        let max_sol_cost = if expected_sol_cost == u64::MAX {
            u64::MAX
        } else {
            expected_sol_cost.saturating_add(expected_sol_cost * slippage_bps / 10_000)
        };

        let expected_sol_out = self.sol_for_tokens(token_amount);
        let min_sol_output = expected_sol_out
            .saturating_sub(expected_sol_out * slippage_bps / 10_000);

        (max_sol_cost, min_sol_output)
    }
}
