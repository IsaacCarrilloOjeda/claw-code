# `trading/` — Ghost alpha module (Python)

This is a **sidecar Python subproject** inside the Ghost / claw-code repo.
Everything in `trading/` is isolated from the Rust CLI and React dashboard —
no shared state, no imports across the boundary. The main Rust daemon will
eventually call this over HTTP (or subprocess) as just another tool.

## What it is

A signal-validation harness for alt-data-driven equity trades. v0 scope: the
Chipotle / CAVA pair. It scrapes public data (job postings, web archives,
eventually Google Trends + reviews), aggregates to weekly features, and tests
whether those features predict the sign of next-quarter revenue surprise.

Only after signals clear a statistical threshold does any trading happen — and
even then via Alpaca paper trading first.

This is **not** a "let Ghost auto-trade" module. It's the validation
scaffolding that decides whether that would ever be a good idea.

## Integration path with the rest of Ghost

v0 (now): standalone Python, SQLite, runs via Windows Task Scheduler.

v1 (after signals validate): expose three HTTP endpoints locally:

- `GET  /nowcast/:ticker` — current predicted surprise + confidence
- `GET  /signals/today` — any tickers crossing an entry threshold
- `POST /trades/execute` — gated, paper-only by default

The Rust daemon at `127.0.0.1:7878` calls these via `reqwest`. No schema
coupling — the trading module owns its SQLite, the daemon owns its Postgres.

v2: if (and only if) a signal clearly works, port the winning feature
extractor to Rust and fold it into the daemon. Python stays for research;
Rust runs production.

## Package layout

```
trading/
├── ghost/                   # Python package — lifts into anywhere
│   ├── config.py            # tickers, CIKs, Workday slots
│   ├── db.py                # sqlite connection + bootstrap
│   ├── scrapers/
│   │   └── jobs_workday.py  # v0 data source: Workday careers API
│   ├── backfill/
│   │   └── wayback.py       # CDX client for historical captures
│   ├── broker/
│   │   └── alpaca.py        # paper quotes, orders, fill sync
│   └── validate/
│       └── correlate.py     # Spearman rho + sign-agreement pass/fail
├── scripts/
│   ├── init_db.py
│   └── scrape_daily.py
├── schema.sql
├── requirements.txt
├── .env.example
└── .gitignore               # ignores data/*.db + .env
```

## First-run

```bash
cd trading
python -m venv .venv
source .venv/Scripts/activate     # Git Bash on Windows
pip install -r requirements.txt
cp .env.example .env              # fill in Alpaca keys
python scripts/init_db.py
```

## Week-1 task before first scrape

`ghost/config.py` has `workday.tenant` / `workday.site` as `None` for both
tickers. Discover them once:

1. Open https://jobs.chipotle.com and https://careers.cava.com
2. DevTools → Network → reload → filter XHR for `/jobs`
3. Copy the tenant and site slugs from the POST URL into `COMPANIES[ticker]["workday"]`
4. `python scripts/scrape_daily.py` — should print `[ok] CMG: N postings ingested`
5. Schedule daily via Windows Task Scheduler

## Validation threshold (before any real money)

- ≥ 8 quarters of earnings outcomes loaded per ticker
- ≥ 26 weeks of snapshots (live + Wayback backfill)
- At least one feature with `|Spearman rho| > 0.3` AND `sign_agree > 0.65`
- 100+ paper-traded positions logged via Alpaca, Sharpe > 1
- One earnings report whose surprise direction the nowcast called correctly
  _in advance_

When all true, flip `ALPACA_PAPER=0` — but start with 10% of intended
capital, not 100%.

## What's intentionally NOT built yet

- Feature aggregation and nowcast model — write after real events rows exist
- Trends / reviews scrapers — one stream at a time, validate before adding
- Pre-announcement consensus ingestion — `yfinance` is current-only; historical
  needs Zacks Wayback scraping (~1–2 days of work)
- HTTP endpoints for daemon integration — only build those once a signal exists

## Isolation rules (so this never pollutes the main codebase)

- No Python imports from `../rust/` or `../dashboard/`
- No writes outside `trading/data/`
- No shared env vars with the Rust daemon — trading owns `ALPACA_*`, the daemon
  owns `GHOST_DAEMON_KEY` and friends
- `data/*.db`, `.env` git-ignored (see `trading/.gitignore`)

## See also

- [`PASSIVE_PLAN.md`](PASSIVE_PLAN.md) — the 80% of capital that is actually
  passive (VOO / SGOV / SCHD-in-Roth). Trading is the 20% sleeve, not the
  whole portfolio.
