import numpy as np
import pandas as pd
from dataclasses import dataclass, field
from typing import List, Optional, Tuple
from enum import Enum
import structlog
import os
import time
import threading

logger = structlog.get_logger(__name__)

try:
    import joblib
    _JOBLIB_AVAILABLE = True
except ImportError:
    import pickle as joblib  # fallback
    _JOBLIB_AVAILABLE = False


class SignalDirection(str, Enum):
    BUY = "BUY"
    SELL = "SELL"
    HOLD = "HOLD"


@dataclass
class Signal:
    direction: SignalDirection
    confidence: float
    reason: str
    token_mint: str
    price: float
    market_cap_sol: float
    liquidity_sol: float
    bonding_curve_progress: float
    volume_score: float = 0.0
    momentum_score: float = 0.0
    ml_score: float = 0.0


class FeatureExtractor:
    """Extract features from token market data for ML model."""

    def extract_features(
        self,
        prices: List[float],
        volumes: List[float],
        liquidity: float,
        market_cap: float,
        bonding_curve_progress: float,
        holder_count: int,
    ) -> np.ndarray:
        prices = np.array(prices, dtype=np.float64)
        volumes = np.array(volumes, dtype=np.float64)

        if len(prices) < 3:
            return self._zero_features()

        features = []

        # Price momentum features
        returns = np.diff(prices) / (prices[:-1] + 1e-10)
        features.append(returns[-1] if len(returns) > 0 else 0.0)
        features.append(np.mean(returns) if len(returns) > 0 else 0.0)
        features.append(np.std(returns) if len(returns) > 1 else 0.0)

        # Short/long momentum
        short_window = min(5, len(prices))
        long_window = min(20, len(prices))
        short_ma = np.mean(prices[-short_window:])
        long_ma = np.mean(prices[-long_window:])
        features.append(float(short_ma / (long_ma + 1e-10) - 1.0))

        # RSI approximation (simplified)
        if len(returns) >= 14:
            gains = returns[-14:][returns[-14:] > 0]
            losses = -returns[-14:][returns[-14:] < 0]
            avg_gain = np.mean(gains) if len(gains) > 0 else 0.0
            avg_loss = np.mean(losses) if len(losses) > 0 else 0.0
            rsi = 100 - (100 / (1 + avg_gain / (avg_loss + 1e-10)))
            features.append(rsi / 100.0)
        else:
            features.append(0.5)

        # Volume features
        if len(volumes) >= 2:
            vol_change = float(volumes[-1] / (np.mean(volumes[:-1]) + 1e-10))
            features.append(min(vol_change, 10.0))
        else:
            features.append(1.0)

        # Market features
        features.append(min(liquidity / 100.0, 1.0))
        features.append(min(market_cap / 1000.0, 1.0))
        features.append(bonding_curve_progress / 100.0)
        features.append(min(holder_count / 1000.0, 1.0))

        # Volatility
        if len(returns) > 2:
            features.append(float(np.std(returns)))
        else:
            features.append(0.0)

        return np.array(features, dtype=np.float64)

    def _zero_features(self) -> np.ndarray:
        return np.zeros(11, dtype=np.float64)

    def feature_size(self) -> int:
        return 11


class MLSignalGenerator:
    """ML-based signal generator using scikit-learn Random Forest."""

    def __init__(self, model_path: str = "models/signal_model.joblib",
                 save_interval_seconds: Optional[float] = None):
        self.model_path = model_path
        self._legacy_path = model_path.replace(".joblib", ".pkl")
        self.model = None
        self.feature_extractor = FeatureExtractor()
        self._lock = threading.Lock()
        self._last_saved: Optional[float] = None
        self._last_checkpoint: Optional[float] = None

        if save_interval_seconds is not None:
            self._save_interval = save_interval_seconds
        else:
            try:
                from config import settings as _settings
                self._save_interval = _settings.ml_save_interval_seconds
            except Exception:
                self._save_interval = 300.0

        self._try_load_model()

    def _try_load_model(self):
        """
        Try to load a pre-trained model from disk, falling back to rule-based signals.

        Load strategy (consistent with _persist_model):
        - When joblib is available, always use joblib.load() regardless of file extension.
          joblib.dump() writes a joblib-compressed format that is NOT reliably readable
          by pickle.load(), so we must use the same library for both read and write.
        - When joblib is unavailable, use pickle.load() as a fallback.
        Legacy .pkl paths are also attempted so that models saved before the .joblib
        rename can still be loaded.
        """
        paths_to_try = [self.model_path, self._legacy_path]
        for path in paths_to_try:
            if not os.path.exists(path):
                continue
            try:
                if _JOBLIB_AVAILABLE:
                    model = joblib.load(path)
                    backend = "joblib"
                else:
                    import pickle
                    with open(path, "rb") as f:
                        model = pickle.load(f)
                    backend = "pickle"
                self.model = model
                logger.info("ML model loaded", path=path, backend=backend)
                return
            except Exception as e:
                logger.warning("Failed to load ML model, trying next path", path=path, error=str(e))
        logger.info("No ML model found, using rule-based signals")

    def generate_signal(
        self,
        token_mint: str,
        price: float,
        price_history: List[float],
        volume_history: List[float],
        liquidity_sol: float,
        market_cap_sol: float,
        bonding_curve_progress: float,
        holder_count: int,
    ) -> Signal:
        """Generate a trading signal for the given token."""
        features = self.feature_extractor.extract_features(
            price_history,
            volume_history,
            liquidity_sol,
            market_cap_sol,
            bonding_curve_progress,
            holder_count,
        )

        if self.model is not None:
            return self._ml_signal(token_mint, price, features, market_cap_sol, liquidity_sol, bonding_curve_progress)
        else:
            return self._rule_based_signal(token_mint, price, features, price_history, volume_history, market_cap_sol, liquidity_sol, bonding_curve_progress)

    def _ml_signal(
        self,
        token_mint: str,
        price: float,
        features: np.ndarray,
        market_cap_sol: float,
        liquidity_sol: float,
        bonding_curve_progress: float,
    ) -> Signal:
        """Generate signal using trained ML model."""
        try:
            proba = self.model.predict_proba([features])[0]
            classes = self.model.classes_
            class_proba = dict(zip(classes, proba))

            buy_prob = class_proba.get("BUY", 0.0)
            sell_prob = class_proba.get("SELL", 0.0)
            hold_prob = class_proba.get("HOLD", 1.0)

            if buy_prob > sell_prob and buy_prob > hold_prob:
                direction = SignalDirection.BUY
                confidence = float(buy_prob)
            elif sell_prob > buy_prob and sell_prob > hold_prob:
                direction = SignalDirection.SELL
                confidence = float(sell_prob)
            else:
                direction = SignalDirection.HOLD
                confidence = float(hold_prob)

            return Signal(
                direction=direction,
                confidence=confidence,
                reason=f"ML model: buy={buy_prob:.2f} sell={sell_prob:.2f}",
                token_mint=token_mint,
                price=price,
                market_cap_sol=market_cap_sol,
                liquidity_sol=liquidity_sol,
                bonding_curve_progress=bonding_curve_progress,
                ml_score=buy_prob if direction == SignalDirection.BUY else sell_prob,
            )
        except Exception as e:
            logger.error("ML model prediction failed", error=str(e))
            return self._rule_based_signal(token_mint, price, features, [], [], market_cap_sol, liquidity_sol, bonding_curve_progress)

    def _rule_based_signal(
        self,
        token_mint: str,
        price: float,
        features: np.ndarray,
        price_history: List[float],
        volume_history: List[float],
        market_cap_sol: float,
        liquidity_sol: float,
        bonding_curve_progress: float,
    ) -> Signal:
        """Generate signal using rule-based approach (fallback when no ML model)."""
        reasons = []
        score = 0.0

        if len(features) >= 11:
            last_return = features[0]
            mean_return = features[1]
            momentum_score = features[3]
            rsi = features[4]
            vol_spike = features[5]

            # Momentum rules
            if last_return > 0.03:
                score += 0.25
                reasons.append(f"+3% recent price")
            if mean_return > 0.01:
                score += 0.15
                reasons.append("positive trend")
            if momentum_score > 0.05:
                score += 0.20
                reasons.append("short MA > long MA")

            # RSI rules (prefer not overbought)
            if 0.3 < rsi < 0.7:
                score += 0.10
                reasons.append("healthy RSI")
            elif rsi > 0.8:
                score -= 0.15
                reasons.append("overbought RSI")

            # Volume spike
            if vol_spike > 2.0:
                score += 0.20
                reasons.append(f"volume spike {vol_spike:.1f}x")

        # Market cap filter
        if 5.0 < market_cap_sol < 500.0:
            score += 0.15
            reasons.append("good market cap range")
        elif market_cap_sol > 500.0:
            score -= 0.10

        # Liquidity filter
        if liquidity_sol > 2.0:
            score += 0.10
        else:
            score -= 0.20
            reasons.append("low liquidity risk")

        # Bonding curve stage
        if 10 < bonding_curve_progress < 80:
            score += 0.10

        score = max(0.0, min(1.0, score))

        if score > 0.55:
            direction = SignalDirection.BUY
            confidence = score
        elif score < 0.25:
            direction = SignalDirection.SELL
            confidence = 1.0 - score
        else:
            direction = SignalDirection.HOLD
            confidence = 0.5

        return Signal(
            direction=direction,
            confidence=confidence,
            reason="; ".join(reasons) if reasons else "neutral",
            token_mint=token_mint,
            price=price,
            market_cap_sol=market_cap_sol,
            liquidity_sol=liquidity_sol,
            bonding_curve_progress=bonding_curve_progress,
            ml_score=score,
        )

    def train(self, X: np.ndarray, y: np.ndarray):
        """Train the ML model on labeled data and persist it using joblib."""
        from sklearn.ensemble import RandomForestClassifier
        from sklearn.preprocessing import StandardScaler
        from sklearn.pipeline import Pipeline

        model = Pipeline([
            ("scaler", StandardScaler()),
            ("clf", RandomForestClassifier(
                n_estimators=100,
                max_depth=5,
                min_samples_split=10,
                random_state=42,
                class_weight="balanced",
            )),
        ])
        model.fit(X, y)
        with self._lock:
            self.model = model
            self._persist_model(model)

    def _persist_model(self, model):
        """Save model to disk using joblib (falls back to pickle)."""
        os.makedirs(os.path.dirname(self.model_path) or ".", exist_ok=True)
        try:
            if _JOBLIB_AVAILABLE:
                joblib.dump(model, self.model_path, compress=3)
                logger.info("ML model saved (joblib)", path=self.model_path)
            else:
                import pickle
                with open(self.model_path, "wb") as f:
                    pickle.dump(model, f)
                logger.info("ML model saved (pickle fallback)", path=self.model_path)
            self._last_saved = time.monotonic()
        except Exception as e:
            logger.error("Failed to persist ML model", error=str(e))

    def maybe_checkpoint(self) -> bool:
        """
        Persist the current model if the save interval has elapsed since the last save.
        Returns True if a checkpoint was written, False otherwise.
        Called periodically by the strategy engine so the model survives restarts
        even between training runs.
        """
        if self.model is None:
            return False
        now = time.monotonic()
        last = self._last_checkpoint or self._last_saved
        if last is not None and (now - last) < self._save_interval:
            return False
        with self._lock:
            if self.model is None:
                return False
            logger.debug(
                "Periodic ML model checkpoint",
                path=self.model_path,
                interval_seconds=self._save_interval,
            )
            self._persist_model(self.model)
            self._last_checkpoint = time.monotonic()
        return True

    def reload_if_stale(self, max_age_seconds: float = 3600.0):
        """Reload the model from disk if it has been updated externally."""
        if not os.path.exists(self.model_path):
            return
        try:
            mtime = os.path.getmtime(self.model_path)
            if self._last_saved is None or mtime > self._last_saved:
                with self._lock:
                    self._try_load_model()
                    logger.info("ML model reloaded from disk", path=self.model_path)
        except Exception as e:
            logger.warning("Could not check model staleness", error=str(e))
