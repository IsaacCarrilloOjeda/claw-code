-- Filesystem watcher toggle for the coder file index. Default `true` so local
-- dev boxes get incremental re-indexing on file changes. On cloud deploys
-- (Railway) the watcher silently skips at spawn when the resolved repo_root
-- doesn't exist, so leaving this `true` is safe.

INSERT INTO settings_kv (key, value) VALUES
    ('coder.index_watcher_enabled', 'true'::jsonb),
    ('coder.repo_root',             '""'::jsonb)
ON CONFLICT (key) DO NOTHING;
