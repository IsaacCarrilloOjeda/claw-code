"""One-shot: apply schema.sql to data/ghost.db."""
from ghost.db import init_db
from ghost.config import DB_PATH

if __name__ == "__main__":
    init_db()
    print(f"Initialized {DB_PATH}")
