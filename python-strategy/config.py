from pydantic_settings import BaseSettings
from pydantic import Field
from typing import Optional


class Settings(BaseSettings):
    environment: str = Field(default="development")

    # gRPC connection to Rust engine
    grpc_host: str = Field(default="localhost")
    grpc_port: int = Field(default=50051)

    # FastAPI metrics server
    api_host: str = Field(default="0.0.0.0")
    api_port: int = Field(default=8001)

    # Redis
    redis_url: str = Field(default="redis://localhost:6379")

    # Strategy parameters
    sniper_enabled: bool = Field(default=True)
    momentum_enabled: bool = Field(default=True)
    ml_enabled: bool = Field(default=True)

    # Sniper strategy settings
    sniper_buy_amount_sol: float = Field(default=0.05)
    sniper_slippage_bps: int = Field(default=500)
    sniper_max_market_cap_sol: float = Field(default=100.0)
    sniper_min_liquidity_sol: float = Field(default=1.0)

    # Momentum strategy settings
    momentum_buy_amount_sol: float = Field(default=0.1)
    momentum_slippage_bps: int = Field(default=300)
    momentum_volume_threshold: float = Field(default=10.0)
    momentum_price_change_threshold: float = Field(default=0.05)
    momentum_window_seconds: int = Field(default=300)

    # ML model settings
    ml_model_path: str = Field(default="models/signal_model.joblib")
    ml_confidence_threshold: float = Field(default=0.65)
    ml_feature_window: int = Field(default=50)

    # Risk management
    max_position_size_sol: float = Field(default=1.0)
    max_portfolio_exposure_sol: float = Field(default=10.0)
    max_daily_loss_sol: float = Field(default=2.0)
    stop_loss_percentage: float = Field(default=0.20)
    take_profit_percentage: float = Field(default=0.50)

    # Polling intervals
    market_scan_interval_seconds: int = Field(default=5)
    position_update_interval_seconds: int = Field(default=10)

    # Circuit breaker
    cb_failure_threshold: int = Field(default=5)
    cb_recovery_interval_seconds: float = Field(default=30.0)

    # ML model persistence
    ml_save_interval_seconds: float = Field(default=300.0)

    # Rolling buffer checkpointing
    buffer_checkpoint_interval_seconds: float = Field(default=300.0)
    buffer_checkpoint_dir: str = Field(default="checkpoints")

    class Config:
        env_file = ".env"
        env_file_encoding = "utf-8"
        extra = "ignore"


settings = Settings()
