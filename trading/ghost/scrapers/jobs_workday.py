"""
Workday jobs scraper.

Workday hosts careers pages for thousands of public companies and exposes an
undocumented but stable JSON endpoint:

    POST https://{tenant}.wd5.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs
    { "appliedFacets": {}, "limit": 20, "offset": 0, "searchText": "" }

To find {tenant} and {site} for a given company:
  1. Open their careers page in a browser
  2. DevTools -> Network tab
  3. Refresh. Filter XHR for "/jobs"
  4. The POST URL reveals the tenant and site slugs
  5. Some companies are on wd1/wd3/wd5 subdomains; adjust if 404

Response shape (partial):
    { "total": 123, "jobPostings": [
        { "title": "...", "locationsText": "...", "postedOn": "Posted 3 Days Ago",
          "externalPath": "/job/...-R-12345", "bulletFields": [...] }, ... ] }

One scrape = snapshot of currently-open requisitions. Time-series (active count
per week, posting velocity, role mix) is derived from `events` table.
"""
from __future__ import annotations

import json
import datetime as dt
import httpx

from ghost.config import COMPANIES
from ghost.db import get_conn


def _endpoint(tenant: str, site: str, subdomain: str = "wd5") -> str:
    return f"https://{tenant}.{subdomain}.myworkdayjobs.com/wday/cxs/{tenant}/{site}/jobs"


def fetch_all(ticker: str, subdomain: str = "wd5") -> list[dict]:
    cfg = COMPANIES[ticker]["workday"]
    if not cfg.get("tenant") or not cfg.get("site"):
        raise RuntimeError(
            f"{ticker}: Workday tenant/site not configured. "
            f"See ghost/config.py and the discovery instructions in this file."
        )
    url = _endpoint(cfg["tenant"], cfg["site"], subdomain)
    out, offset, limit = [], 0, 20
    with httpx.Client(timeout=30, headers={"Accept": "application/json"}) as c:
        while True:
            r = c.post(
                url,
                json={"appliedFacets": {}, "limit": limit, "offset": offset, "searchText": ""},
            )
            r.raise_for_status()
            data = r.json()
            postings = data.get("jobPostings", [])
            if not postings:
                break
            out.extend(postings)
            offset += limit
            if offset >= data.get("total", 0):
                break
    return out


def ingest(ticker: str) -> int:
    now = dt.datetime.utcnow()
    postings = fetch_all(ticker)
    rows = [
        (ticker, now.isoformat(), now.isoformat(), p.get("externalPath"), json.dumps(p))
        for p in postings
    ]
    with get_conn() as con:
        con.executemany(
            """INSERT OR IGNORE INTO events
               (ticker, stream, observed_at, scraped_at, external_id, payload)
               VALUES (?, 'jobs', ?, ?, ?, ?)""",
            rows,
        )
    return len(rows)
