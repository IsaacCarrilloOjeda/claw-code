from pathlib import Path
from dotenv import load_dotenv

load_dotenv()

ROOT = Path(__file__).resolve().parents[1]
DATA_DIR = ROOT / "data"
DB_PATH = DATA_DIR / "ghost.db"
SCHEMA_PATH = ROOT / "schema.sql"

# Pair under study. Extend once v0 validates.
TICKERS = ["CMG", "CAVA"]

COMPANIES = {
    "CMG": {
        "name": "Chipotle Mexican Grill",
        "cik": "0001058090",
        "careers_url": "https://jobs.chipotle.com/",
        # Verify by opening careers page, DevTools -> Network, find the POST to
        # /wday/cxs/{tenant}/{site}/jobs. Fill in tenant + site below.
        "workday": {"tenant": None, "site": None},
        "trends_terms": ["chipotle", "chipotle near me"],
    },
    "CAVA": {
        "name": "CAVA Group",
        "cik": "0001869794",
        "careers_url": "https://careers.cava.com/",
        "workday": {"tenant": None, "site": None},
        "trends_terms": ["cava", "cava near me"],
    },
}

DATA_DIR.mkdir(parents=True, exist_ok=True)
