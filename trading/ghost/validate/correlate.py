"""
Signal validation. Run AFTER backfilling at least 8 quarters of snapshots +
earnings outcomes. Passes a feature iff |Spearman rho| > 0.3 AND sign-agreement
> 65% AND n >= 8. Anything below = drop the feature.
"""
from __future__ import annotations

import pandas as pd
from ghost.db import get_conn


THRESH_RHO = 0.30
THRESH_SIGN_AGREE = 0.65
MIN_N = 8


def _load() -> tuple[pd.DataFrame, pd.DataFrame]:
    with get_conn() as con:
        snaps = pd.read_sql("SELECT * FROM snapshots", con)
        earn = pd.read_sql("SELECT * FROM earnings", con)
    return snaps, earn


def _window_feature(
    snaps: pd.DataFrame,
    ticker: str,
    metric: str,
    report_date: pd.Timestamp,
    lag_weeks_start: int = 8,
    lag_weeks_end: int = 2,
) -> float | None:
    start = report_date - pd.Timedelta(weeks=lag_weeks_start)
    end = report_date - pd.Timedelta(weeks=lag_weeks_end)
    window = snaps[
        (snaps.ticker == ticker)
        & (snaps.metric == metric)
        & (pd.to_datetime(snaps.week_ending) >= start)
        & (pd.to_datetime(snaps.week_ending) <= end)
    ]
    if len(window) < 3:
        return None
    return float(window.value.mean())


def correlate_all() -> pd.DataFrame:
    snaps, earn = _load()
    if snaps.empty or earn.empty:
        print("No snapshots or earnings yet. Backfill first.")
        return pd.DataFrame()
    earn["report_date"] = pd.to_datetime(earn.report_date)
    results = []
    for (ticker, metric), _ in snaps.groupby(["ticker", "metric"]):
        pairs = []
        for _, row in earn[earn.ticker == ticker].iterrows():
            x = _window_feature(snaps, ticker, metric, row.report_date)
            y = row.revenue_surprise_pct
            if x is None or pd.isna(y):
                continue
            pairs.append((x, y))
        if len(pairs) < MIN_N:
            continue
        df = pd.DataFrame(pairs, columns=["x", "y"])
        rho = df.x.corr(df.y, method="spearman")
        sign_agree = ((df.x - df.x.median()).gt(0) == df.y.gt(0)).mean()
        passes = abs(rho) >= THRESH_RHO and sign_agree >= THRESH_SIGN_AGREE
        results.append(
            {
                "ticker": ticker,
                "metric": metric,
                "n": len(pairs),
                "spearman_rho": round(rho, 3),
                "sign_agree": round(sign_agree, 3),
                "passes": passes,
            }
        )
    return pd.DataFrame(results).sort_values("spearman_rho", key=abs, ascending=False)


if __name__ == "__main__":
    df = correlate_all()
    if not df.empty:
        print(df.to_string(index=False))
        print(f"\n{df.passes.sum()}/{len(df)} features pass threshold.")
