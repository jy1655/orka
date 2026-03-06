CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  channel TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active',
  last_seen_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS event_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  idempotency_key TEXT,
  channel TEXT NOT NULL,
  direction TEXT NOT NULL,
  chat_id TEXT NOT NULL,
  user_id TEXT,
  payload_text TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_event_log_idempotency
ON event_log(idempotency_key)
WHERE idempotency_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS command_state (
  scope_key TEXT PRIMARY KEY,
  paused INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL
);
