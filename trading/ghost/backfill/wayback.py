"""
Wayback Machine CDX client.

Given a URL, list every archived capture, then re-fetch the captures to extract
historical state (e.g. careers-page job count in 2021). Free, rate-limited.

Expect ~40% of captures to be broken/partial. Skip, don't fix.
"""
from __future__ import annotations

import time
import httpx

CDX_URL = "https://web.archive.org/cdx/search/cdx"


def list_snapshots(
    url: str,
    from_ym: str = "201906",
    to_ym: str | None = None,
    collapse_days: int = 1,
) -> list[tuple[str, str]]:
    """Return [(timestamp, wayback_url)] for captures of the given URL."""
    params = {
        "url": url,
        "from": from_ym + "01",
        "to": (to_ym or time.strftime("%Y%m")) + "31",
        "output": "json",
        "filter": "statuscode:200",
        "collapse": f"timestamp:{8 if collapse_days == 1 else 10}",
    }
    r = httpx.get(CDX_URL, params=params, timeout=60)
    r.raise_for_status()
    rows = r.json()
    return [
        (row[1], f"https://web.archive.org/web/{row[1]}/{row[2]}")
        for row in rows[1:]  # skip header
    ]


def fetch_capture(wayback_url: str) -> str | None:
    try:
        r = httpx.get(wayback_url, timeout=45, follow_redirects=True)
        if r.status_code == 200:
            return r.text
    except httpx.HTTPError:
        pass
    return None
