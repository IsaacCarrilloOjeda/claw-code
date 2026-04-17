-- Scholar DB: caches successful solutions, tracks failed approaches.
-- Prevents repeated mistakes and skips API calls for solved problems.

CREATE TABLE IF NOT EXISTS scholar_solutions (
    id              UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    problem_sig     TEXT NOT NULL,
    problem_embed   VECTOR(1024),
    failed_attempts TEXT[] DEFAULT '{}',
    solution        TEXT NOT NULL,
    solution_lang   TEXT,
    context_file    TEXT,
    success_count   INTEGER NOT NULL DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
