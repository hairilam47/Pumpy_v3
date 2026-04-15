use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tracing::{info, warn, error};

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
///
/// Thread-safe state:
/// - `consecutive_rejects`: counts unbroken REJECT streak per wallet instance.
///   Reset to 0 on any ALLOW.
/// - `auto_paused`: latched true once `auto_pause_threshold` consecutive REJECTs
///   are observed. Once latched, all subsequent evaluate() calls return Halt.
/// - `needs_db_pause`: single-fire flag that the OrderManager reads after each
///   evaluate() to kick off an async DB status update to `paused`.
pub struct DecisionEngine {
    consecutive_rejects: AtomicU32,
    auto_paused: AtomicBool,
    /// Single-fire: set true when auto_pause is first triggered.
    /// `take_needs_db_pause()` clears it and returns the old value.
    needs_db_pause: AtomicBool,
    auto_pause_threshold: u32,
}

impl DecisionEngine {
    pub fn new() -> Self {
        Self::with_threshold(10)
    }

    pub fn with_threshold(auto_pause_threshold: u32) -> Self {
        Self {
            consecutive_rejects: AtomicU32::new(0),
            auto_paused: AtomicBool::new(false),
            needs_db_pause: AtomicBool::new(false),
            auto_pause_threshold,
        }
    }

    /// Returns true (and resets the flag) if this engine just crossed the
    /// auto-pause threshold for the first time. The caller should
    /// async-update the database wallet status to 'paused'.
    pub fn take_needs_db_pause(&self) -> bool {
        self.needs_db_pause.swap(false, Ordering::AcqRel)
    }

    /// Returns true if this engine is in auto-paused state.
    pub fn is_auto_paused(&self) -> bool {
        self.auto_paused.load(Ordering::Acquire)
    }

    /// Reset all auto-pause state. Call when an operator manually resumes a wallet.
    /// Clears the consecutive-reject counter, lifts the auto_paused latch, and
    /// discards any pending needs_db_pause flag so the next order can be evaluated
    /// cleanly.
    pub fn reset_pause(&self) {
        self.consecutive_rejects.store(0, Ordering::Release);
        self.auto_paused.store(false, Ordering::Release);
        self.needs_db_pause.store(false, Ordering::Release);
    }

    /// Evaluate a trade request and return the authoritative Decision.
    ///
    /// Rules applied in priority order:
    ///   0. Auto-pause guard (Halt — if N consecutive REJECTs exceeded)
    ///   1. Demo mode guard (Halt)
    ///   2. Basic parameter validation (Reject)
    ///   3. Position-size risk check (Reject)
    ///   4. Portfolio exposure limit (Reject)
    ///   5. Daily loss limit (Reject)
    ///   6. MEV / sandwich risk check (Reject)
    ///   7. Allow (resets consecutive_rejects)
    pub fn evaluate(&self, ctx: &DecisionContext<'_>) -> Decision {
        let order = ctx.order;

        if self.auto_paused.load(Ordering::Acquire) {
            return self.emit(ctx, Decision::Halt {
                reason: format!(
                    "wallet auto-paused after {} consecutive rejections; \
                     resume via the Wallets page or admin API",
                    self.auto_pause_threshold
                ),
            });
        }

        if ctx.demo_mode {
            return self.emit(ctx, Decision::Halt {
                reason: "no real wallet configured (demo mode); \
                         set WALLET_PRIVATE_KEY or KEYPAIR_PATH to enable live trading"
                    .to_string(),
            });
        }

        if order.amount == 0 {
            return self.on_reject(ctx, Decision::Reject {
                reason: "amount must be greater than zero".to_string(),
            });
        }

        if order.slippage_bps > ctx.max_slippage_bps {
            return self.on_reject(ctx, Decision::Reject {
                reason: format!(
                    "slippage_bps={} exceeds allowed maximum of {}",
                    order.slippage_bps, ctx.max_slippage_bps
                ),
            });
        }

        let trade_sol = order.amount as f64 / 1_000_000_000.0;
        if trade_sol > ctx.max_position_size_sol {
            return self.on_reject(ctx, Decision::Reject {
                reason: format!(
                    "trade size {:.4} SOL exceeds max position size {:.4} SOL",
                    trade_sol, ctx.max_position_size_sol
                ),
            });
        }

        let projected_exposure = ctx.current_portfolio_exposure_sol + trade_sol;
        if projected_exposure > ctx.max_portfolio_exposure_sol {
            return self.on_reject(ctx, Decision::Reject {
                reason: format!(
                    "projected portfolio exposure {:.4} SOL would exceed limit {:.4} SOL",
                    projected_exposure, ctx.max_portfolio_exposure_sol
                ),
            });
        }

        if ctx.current_daily_loss_sol > ctx.max_daily_loss_sol {
            return self.on_reject(ctx, Decision::Reject {
                reason: format!(
                    "daily loss {:.4} SOL has exceeded limit {:.4} SOL",
                    ctx.current_daily_loss_sol, ctx.max_daily_loss_sol
                ),
            });
        }

        if ctx.sandwich_risk_score > ctx.max_sandwich_risk_score {
            return self.on_reject(ctx, Decision::Reject {
                reason: format!(
                    "sandwich risk score={} exceeds max={}",
                    ctx.sandwich_risk_score, ctx.max_sandwich_risk_score
                ),
            });
        }

        self.consecutive_rejects.store(0, Ordering::Release);
        self.emit(ctx, Decision::Allow)
    }

    /// Called for every REJECT decision. Updates the consecutive reject counter
    /// and triggers auto-pause if the threshold is crossed.
    fn on_reject(&self, ctx: &DecisionContext<'_>, decision: Decision) -> Decision {
        let prev = self.consecutive_rejects.fetch_add(1, Ordering::AcqRel);
        let count = prev + 1;

        if count >= self.auto_pause_threshold {
            let was_paused = self.auto_paused.swap(true, Ordering::AcqRel);
            if !was_paused {
                self.needs_db_pause.store(true, Ordering::Release);
                error!(
                    decision = "HALT",
                    wallet_id = ctx.wallet_id,
                    consecutive_rejects = count,
                    threshold = self.auto_pause_threshold,
                    "DecisionEngine: AUTO_PAUSE — wallet paused after consecutive rejections"
                );
                return self.emit(ctx, Decision::Halt {
                    reason: format!(
                        "wallet auto-paused after {} consecutive rejections; \
                         resume via the Wallets page or admin API",
                        self.auto_pause_threshold
                    ),
                });
            }
        } else {
            info!(
                decision = "REJECT",
                wallet_id = ctx.wallet_id,
                consecutive_rejects = count,
                threshold = self.auto_pause_threshold,
                "DecisionEngine: consecutive reject count"
            );
        }

        self.emit(ctx, decision)
    }

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
