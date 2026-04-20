"""Daily scrape. Wire to Windows Task Scheduler once Workday tenants are set."""
from ghost.config import TICKERS, COMPANIES
from ghost.scrapers.jobs_workday import ingest as ingest_jobs


def main():
    for ticker in TICKERS:
        wd = COMPANIES[ticker]["workday"]
        if not wd.get("tenant"):
            print(f"[skip] {ticker}: Workday tenant not configured yet")
            continue
        try:
            n = ingest_jobs(ticker)
            print(f"[ok]   {ticker}: {n} postings ingested")
        except Exception as e:
            print(f"[err]  {ticker}: {e}")


if __name__ == "__main__":
    main()
