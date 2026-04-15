/// The single Decision Engine — the ONLY authority that can approve or deny a trade.
///
/// All execution paths must call `evaluate()` and obey the result blindly:
///   `Allow`  — trade is approved, execution may proceed.
///   `Reject` — trade is denied for business/risk reasons; mark order Failed, no retry.
///   `Halt`   — critical safety or config failure; stop the wallet worker.
///
/// No execution code may submit an on-chain transaction without first receiving
/// an `Allow` decision from this engine.
use tracing::{info, warn, error};

use crate::order::Order;

// ─── Decision ────────────────────────────────────────────────────────────────

/// The authoritative outcome for every trade request.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Trade is approved — execution may proceed.
    Allow,
    /// Trade is denied. The order must be marked Failed. No chain interaction occurs.
    Reject { reason: String },
    /// Critical failure — the wallet worker must stop. Operator intervention required.
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

// ─── DecisionContext ──────────────────────────────────────────────────────────

/// All inputs the Decision Engine needs to evaluate a single trade request.
/// Callers must populate every field; the engine treats missing/zero values
/// as policy violations, not errors.
pub struct DecisionContext<'a> {
    /// Identifier for the wallet submitting this order.
    pub wallet_id: &'a str,
    /// The order under evaluation.
    pub order: &'a Order,
    /// True when the engine started without a real wallet (ephemeral keypair).
    pub demo_mode: bool,
    /// Maximum trade size in SOL. Sourced from system config.
    pub max_position_size_sol: f64,
    /// Maximum total portfolio exposure in SOL. Sourced from system config.
    pub max_portfolio_exposure_sol: f64,
    /// Maximum allowed daily loss in SOL. Sourced from system config.
    pub max_daily_loss_sol: f64,
    /// Maximum allowed slippage in basis points. Sourced from system config.
    pub max_slippage_bps: u64,
    /// MEV sandwich risk score computed for this order (0–100).
    /// Pass 0 when evaluated before the sandwich check runs.
    pub max_sandwich_risk_score: u32,
    /// The actual sandwich risk score for this specific order.
    pub sandwich_risk_score: u32,
    /// Opaque string identifying the config snapshot (e.g. a hash or version counter).
    /// Included in every structured log line for auditability.
    pub config_version: &'a str,
}

// ─── DecisionEngine ──────────────────────────────────────────────────────────

/// Single Decision Engine. Instantiate once per wallet worker and share via Arc.
pub struct DecisionEngine;

impl DecisionEngine {
    pub fn new() -> Self {
        Self
    }

    /// Evaluate a trade request and return the authoritative Decision.
    ///
    /// Rules are applied in strict priority order:
    ///   1. Demo mode guard (Halt)
    ///   2. Basic parameter validation (Reject)
    ///   3. Position-size risk check (Reject)
    ///   4. MEV / sandwich risk check (Reject)
    ///   5. Allow
    ///
    /// Every outcome is logged with wallet_id, order_id, decision, reason,
    /// and config_version for a full audit trail.
    pub fn evaluate(&self, ctx: &DecisionContext<'_>) -> Decision {
        let order = ctx.order;

        // ── 1. Demo mode guard ────────────────────────────────────────────
        if ctx.demo_mode {
            let reason = "Trading halted: no real wallet configured (demo mode). \
                          Set WALLET_PRIVATE_KEY or KEYPAIR_PATH to enable live trading."
                .to_string();
            self.emit(ctx, Decision::Halt { reason })
        }
        // ── 2. Basic parameter validation ─────────────────────────────────
        else if order.amount == 0 {
            self.emit(ctx, Decision::Reject {
                reason: "amount must be greater than zero".to_string(),
            })
        } else if order.slippage_bps > ctx.max_slippage_bps {
            self.emit(ctx, Decision::Reject {
                reason: format!(
                    "slippage_bps={} exceeds allowed maximum of {}",
                    order.slippage_bps, ctx.max_slippage_bps
                ),
            })
        }
        // ── 3. Position-size risk check ───────────────────────────────────
        else if (order.amount as f64 / 1_000_000_000.0) > ctx.max_position_size_sol {
            let trade_sol = order.amount as f64 / 1_000_000_000.0;
            self.emit(ctx, Decision::Reject {
                reason: format!(
                    "trade size {:.4} SOL exceeds max position size {:.4} SOL",
                    trade_sol, ctx.max_position_size_sol
                ),
            })
        }
        // ── 4. MEV / sandwich risk check ──────────────────────────────────
        else if ctx.sandwich_risk_score > ctx.max_sandwich_risk_score {
            self.emit(ctx, Decision::Reject {
                reason: format!(
                    "sandwich risk score={} exceeds max={}",
                    ctx.sandwich_risk_score, ctx.max_sandwich_risk_score
                ),
            })
        }
        // ── 5. Allow ──────────────────────────────────────────────────────
        else {
            self.emit(ctx, Decision::Allow)
        }
    }

    /// Log the decision and return it.
    fn emit(&self, ctx: &DecisionContext<'_>, decision: Decision) -> Decision {
        let label = decision.label();
        let reason = decision.reason();

        match &decision {
            Decision::Allow => {
                info!(
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
