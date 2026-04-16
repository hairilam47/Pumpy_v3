pub mod manager;

use std::collections::HashMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use uuid::Uuid;

pub use manager::OrderManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub mint: String,
    pub order_type: OrderType,
    pub side: OrderSide,
    pub amount: u64,
    pub price: Option<f64>,
    pub max_cost: Option<u64>,
    pub min_output: Option<u64>,
    pub slippage_bps: u64,
    pub status: OrderStatus,
    pub strategy: String,
    pub metadata: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub executed_at: Option<DateTime<Utc>>,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub retry_count: u32,
    pub executed_price: Option<f64>,
    pub executed_amount: Option<u64>,
    /// Stable client-assigned UUID for idempotency tracking (Task #26)
    pub client_order_id: Option<Uuid>,
    /// Distributed trace ID propagated from the originating service (Task #31)
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
    TakeProfit,
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderType::Market => write!(f, "MARKET"),
            OrderType::Limit => write!(f, "LIMIT"),
            OrderType::StopLoss => write!(f, "STOP_LOSS"),
            OrderType::TakeProfit => write!(f, "TAKE_PROFIT"),
        }
    }
}

impl FromStr for OrderType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "MARKET" => Ok(OrderType::Market),
            "LIMIT" => Ok(OrderType::Limit),
            "STOP_LOSS" => Ok(OrderType::StopLoss),
            "TAKE_PROFIT" => Ok(OrderType::TakeProfit),
            _ => Err(format!("Unknown order type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

impl std::fmt::Display for OrderSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderSide::Buy => write!(f, "BUY"),
            OrderSide::Sell => write!(f, "SELL"),
        }
    }
}

impl FromStr for OrderSide {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "BUY" => Ok(OrderSide::Buy),
            "SELL" => Ok(OrderSide::Sell),
            _ => Err(format!("Unknown order side: {}", s)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum OrderStatus {
    Pending,
    Validating,
    Executing,
    Executed,
    Failed,
    Cancelled,
    Expired,
}

impl std::fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            OrderStatus::Pending => "Pending",
            OrderStatus::Validating => "Validating",
            OrderStatus::Executing => "Executing",
            OrderStatus::Executed => "Executed",
            OrderStatus::Failed => "Failed",
            OrderStatus::Cancelled => "Cancelled",
            OrderStatus::Expired => "Expired",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    #[error("Invalid amount")]
    InvalidAmount,
    #[error("Slippage too high")]
    SlippageTooHigh,
    #[error("Token not found")]
    TokenNotFound,
    #[error("Position size too large")]
    PositionSizeTooLarge,
    #[error("Portfolio exposure limit exceeded")]
    ExposureLimitExceeded,
    #[error("Daily loss limit exceeded")]
    DailyLossLimitExceeded,
    #[error("Order queue full")]
    QueueFull,
    #[error("Order not found: {0}")]
    OrderNotFound(String),
    #[error("Sandwich risk too high: score={0}")]
    SandwichRiskTooHigh(u32),
    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("Channel error")]
    ChannelError,
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for OrderError {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        OrderError::ChannelError
    }
}
