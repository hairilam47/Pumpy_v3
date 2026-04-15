use tracing::{warn, error};

use crate::order::Order;

/// The authoritative outcome for every trade request.
/// All execution paths must call `evaluate()` and obey this blindly.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Reject { reason: String },
    Halt { reason: String },
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Decision::Allow => "ALLOW",
            Decision::Reject { .. } => "REJECT",
            Decision::Halt { .. } => "HALT",
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            Decision::Allow => "trade approved",
            Decision::Reject { reason } | Decision::Halt { reason } => reason.as_str(),
        }
    }
}

/// All inputs the Decision Engine needs to evaluate a single trade request.
pub struct DecisionContext<'a> {
    pub wallet_id: &'a str,
    pub order: &'a Order,
    pub demo_mode: bool,
    pub max_position_size_sol: f64,
    pub max_portfolio_exposure_sol: f64,
    pub max_daily_loss_sol: f64,
    pub max_slippage_bps: u64,
    pub max_sandwich_risk_score: u32,
    pub sandwich_risk_score: u32,
    /// Current total portfolio exposure in SOL (live value from portfolio tracker).
    pub current_portfolio_exposure_sol: f64,
    /// Current day's realized loss in SOL (positive = loss).
    pub current_daily_loss_sol: f64,
    /// Opaque config snapshot identifier for audit logs.
    pub config_version: &'a str,
}

/// The single Decision Engine — the only authority that can approve or deny a trade.
/// No code may submit an on-chain transaction without first receiving `Allow`.
pub struct DecisionEngine;

impl DecisionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Evaluate a trade request and return the authoritative Decision.
    ///
    /// Rules applied in priority order:
    ///   1. Demo mode guard (Halt)
    ///   2. Basic parameter validation (Reject)
    ///   3. Position-size risk check (Reject)
    ///   4. Portfolio exposure limit (Reject)
    ///   5. Daily loss limit (Reject)
    ///   6. MEV / sandwich risk check (Reject)
    ///   7. Allow
    pub fn evaluate(&self, ctx: &DecisionContext<'_>) -> Decision {
        let order = ctx.order;

        if ctx.demo_mode {
            return self.emit(ctx, Decision::Halt {
                reason: "no real wallet configured (demo mode); \
                         set WALLET_PRIVATE_KEY or KEYPAIR_PATH to enable live trading"
                    .to_string(),
            });
        }

        if order.amount == 0 {
            return self.emit(ctx, Decision::Reject {
                reason: "amount must be greater than zero".to_string(),
            });
        }

        if order.slippage_bps > ctx.max_slippage_bps {
            return self.emit(ctx, Decision::Reject {
                reason: format!(
                    "slippage_bps={} exceeds allowed maximum of {}",
                    order.slippage_bps, ctx.max_slippage_bps
                ),
            });
        }

        let trade_sol = order.amount as f64 / 1_000_000_000.0;
        if trade_sol > ctx.max_position_size_sol {
            return self.emit(ctx, Decision::Reject {
                reason: format!(
                    "trade size {:.4} SOL exceeds max position size {:.4} SOL",
                    trade_sol, ctx.max_position_size_sol
                ),
            });
        }

        let projected_exposure = ctx.current_portfolio_exposure_sol + trade_sol;
        if projected_exposure > ctx.max_portfolio_exposure_sol {
            return self.emit(ctx, Decision::Reject {
                reason: format!(
                    "projected portfolio exposure {:.4} SOL would exceed limit {:.4} SOL",
                    projected_exposure, ctx.max_portfolio_exposure_sol
                ),
            });
        }

        if ctx.current_daily_loss_sol > ctx.max_daily_loss_sol {
            return self.emit(ctx, Decision::Reject {
                reason: format!(
                    "daily loss {:.4} SOL has exceeded limit {:.4} SOL",
                    ctx.current_daily_loss_sol, ctx.max_daily_loss_sol
                ),
            });
        }

        if ctx.sandwich_risk_score > ctx.max_sandwich_risk_score {
            return self.emit(ctx, Decision::Reject {
                reason: format!(
                    "sandwich risk score={} exceeds max={}",
                    ctx.sandwich_risk_score, ctx.max_sandwich_risk_score
                ),
            });
        }

        self.emit(ctx, Decision::Allow)
    }

    fn emit(&self, ctx: &DecisionContext<'_>, decision: Decision) -> Decision {
        let label = decision.label();
        let reason = decision.reason();

        match &decision {
            Decision::Allow => {
                warn!(
                    decision = label,
                    wallet_id = ctx.wallet_id,
                    order_id = ctx.order.id,
                    reason = reason,
                    config_version = ctx.config_version,
                    "DecisionEngine"
                );
            }
            Decision::Reject { .. } => {
                warn!(
                    decision = label,
                    wallet_id = ctx.wallet_id,
                    order_id = ctx.order.id,
                    reason = reason,
                    config_version = ctx.config_version,
                    "DecisionEngine"
                );
            }
            Decision::Halt { .. } => {
                error!(
                    decision = label,
                    wallet_id = ctx.wallet_id,
                    order_id = ctx.order.id,
                    reason = reason,
                    config_version = ctx.config_version,
                    "DecisionEngine"
                );
            }
        }

        decision
    }
}

impl Default for DecisionEngine {
    fn default() -> Self {
        Self::new()
    }
}
