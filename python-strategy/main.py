import asyncio
import signal
import structlog
import logging
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from contextlib import asynccontextmanager

from config import settings
from strategy_engine import StrategyEngine
from api.routes import router

# Configure structured logging
structlog.configure(
    wrapper_class=structlog.make_filtering_bound_logger(logging.INFO),
)
logger = structlog.get_logger(__name__)


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Manage application lifecycle."""
    engine = StrategyEngine()
    app.state.engine = engine

    await engine.start()
    logger.info(
        "PumpFun Strategy Engine API started",
        host=settings.api_host,
        port=settings.api_port,
        grpc_target=f"{settings.grpc_host}:{settings.grpc_port}",
    )

    yield

    logger.info("Shutting down strategy engine")
    await engine.stop()


app = FastAPI(
    title="PumpFun Strategy Engine",
    description="ML-based strategy engine for Pump.fun trading bot",
    version="1.0.0",
    lifespan=lifespan,
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(router, prefix="/api")


@app.get("/")
async def root():
    return {
        "service": "PumpFun Strategy Engine",
        "version": "1.0.0",
        "docs": "/docs",
        "health": "/api/health",
    }


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        app,
        host=settings.api_host,
        port=settings.api_port,
        log_level="info",
    )
