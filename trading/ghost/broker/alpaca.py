"""
Alpaca paper-trading adapter.

Replaces the closed-app sim with a real-market-data paper account. Live trading
is a one-line flip (ALPACA_PAPER=0 in .env) - DO NOT DO THAT until the
validation threshold in README is met.

Docs: https://alpaca.markets/docs/
SDK:  https://github.com/alpacahq/alpaca-py
"""
from __future__ import annotations

import os
import json
from alpaca.trading.client import TradingClient
from alpaca.trading.requests import MarketOrderRequest, GetOrdersRequest
from alpaca.trading.enums import OrderSide, TimeInForce, QueryOrderStatus
from alpaca.data.historical import StockHistoricalDataClient
from alpaca.data.requests import StockLatestQuoteRequest, StockBarsRequest
from alpaca.data.timeframe import TimeFrame

from ghost.db import get_conn


def _is_paper() -> bool:
    return os.environ.get("ALPACA_PAPER", "1") == "1"


def _trading() -> TradingClient:
    return TradingClient(
        api_key=os.environ["ALPACA_API_KEY"],
        secret_key=os.environ["ALPACA_API_SECRET"],
        paper=_is_paper(),
    )


def _data() -> StockHistoricalDataClient:
    return StockHistoricalDataClient(
        api_key=os.environ["ALPACA_API_KEY"],
        secret_key=os.environ["ALPACA_API_SECRET"],
    )


def latest_quotes(symbols: list[str]) -> dict:
    req = StockLatestQuoteRequest(symbol_or_symbols=symbols)
    return _data().get_stock_latest_quote(req)


def daily_bars(symbol: str, start: str, end: str):
    req = StockBarsRequest(
        symbol_or_symbols=symbol, timeframe=TimeFrame.Day, start=start, end=end
    )
    return _data().get_stock_bars(req)


def submit_order(symbol: str, qty: float, side: str, tag: str = "ghost_v0"):
    if not _is_paper():
        raise RuntimeError(
            "ALPACA_PAPER is not set to 1. Refusing to submit live orders "
            "from ghost.broker.alpaca.submit_order - enable explicitly if intended."
        )
    req = MarketOrderRequest(
        symbol=symbol,
        qty=qty,
        side=OrderSide.BUY if side.lower() == "buy" else OrderSide.SELL,
        time_in_force=TimeInForce.DAY,
        client_order_id=f"{tag}-{symbol}-{int(__import__('time').time())}",
    )
    order = _trading().submit_order(req)
    return order


def sync_fills(since_iso: str | None = None) -> int:
    """Pull recent orders from Alpaca into the local fills table."""
    tc = _trading()
    req = GetOrdersRequest(status=QueryOrderStatus.ALL, limit=500, after=since_iso)
    orders = tc.get_orders(filter=req)
    rows = []
    for o in orders:
        d = o.model_dump() if hasattr(o, "model_dump") else dict(o)
        rows.append(
            (
                str(d.get("id")),
                d.get("symbol"),
                str(d.get("side")).split(".")[-1].lower(),
                float(d.get("qty") or 0),
                float(d.get("filled_avg_price") or 0) or None,
                str(d.get("submitted_at") or ""),
                str(d.get("filled_at") or "") or None,
                str(d.get("status")),
                d.get("client_order_id", "").split("-")[0] or "manual",
                json.dumps(d, default=str),
            )
        )
    with get_conn() as con:
        con.executemany(
            """INSERT OR REPLACE INTO fills
               (id, symbol, side, qty, filled_avg_price, submitted_at, filled_at,
                status, strategy_tag, payload)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            rows,
        )
    return len(rows)
