import sqlite3
from contextlib import contextmanager
from ghost.config import DB_PATH, SCHEMA_PATH


def init_db() -> None:
    with sqlite3.connect(DB_PATH) as con:
        con.executescript(SCHEMA_PATH.read_text())


@contextmanager
def get_conn():
    con = sqlite3.connect(DB_PATH, detect_types=sqlite3.PARSE_DECLTYPES)
    con.row_factory = sqlite3.Row
    try:
        yield con
        con.commit()
    finally:
        con.close()
