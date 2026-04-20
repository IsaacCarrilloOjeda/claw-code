-- Ghost v0 schema. SQLite.
-- Design notes:
--   events: append-only raw observations. Never mutated. If aggregation logic
--           changes, rebuild snapshots from events.
--   snapshots: derived weekly features. Rebuildable.
--   earnings: supervised ground truth.
--   nowcasts: model outputs + JSON of inputs used (so you can reproduce later).
--   fills: broker-side trade history (Alpaca paper or live), synced separately.

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY,
    ticker TEXT NOT NULL,
    stream TEXT NOT NULL,            -- 'jobs' | 'trends' | 'reviews' | 'traffic'
    observed_at TIMESTAMP NOT NULL,  -- when the datapoint is valid for
    scraped_at TIMESTAMP NOT NULL,   -- when we recorded it
    external_id TEXT,                -- dedup key (e.g. Workday requisitionId)
    payload TEXT NOT NULL,           -- JSON of full raw record
    UNIQUE(stream, external_id, ticker)
);
CREATE INDEX IF NOT EXISTS idx_events_ticker_stream_obs
    ON events(ticker, stream, observed_at);

CREATE TABLE IF NOT EXISTS snapshots (
    ticker TEXT NOT NULL,
    week_ending DATE NOT NULL,       -- Sunday of ISO week
    stream TEXT NOT NULL,
    metric TEXT NOT NULL,            -- 'jobs_active', 'jobs_delta_4w', ...
    value REAL NOT NULL,
    PRIMARY KEY (ticker, week_ending, stream, metric)
);

CREATE TABLE IF NOT EXISTS earnings (
    ticker TEXT NOT NULL,
    fiscal_period TEXT NOT NULL,     -- '2024Q3'
    report_date DATE NOT NULL,
    revenue_actual REAL,
    revenue_consensus REAL,
    revenue_surprise_pct REAL,
    eps_actual REAL,
    eps_consensus REAL,
    price_reaction_1d REAL,
    PRIMARY KEY (ticker, fiscal_period)
);

CREATE TABLE IF NOT EXISTS nowcasts (
    ticker TEXT NOT NULL,
    week_ending DATE NOT NULL,
    target_period TEXT NOT NULL,
    predicted_surprise REAL,
    confidence REAL,
    model_version TEXT NOT NULL,
    features_snapshot TEXT NOT NULL, -- JSON
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (ticker, week_ending, target_period, model_version)
);

CREATE TABLE IF NOT EXISTS fills (
    id TEXT PRIMARY KEY,             -- broker order id
    symbol TEXT NOT NULL,
    side TEXT NOT NULL,              -- 'buy' | 'sell'
    qty REAL NOT NULL,
    filled_avg_price REAL,
    submitted_at TIMESTAMP,
    filled_at TIMESTAMP,
    status TEXT,
    strategy_tag TEXT,               -- 'ghost_v0' | 'manual'
    payload TEXT
);
CREATE INDEX IF NOT EXISTS idx_fills_symbol_filled ON fills(symbol, filled_at);
