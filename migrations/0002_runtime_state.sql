CREATE TABLE IF NOT EXISTS provider_preferences (
  scope_key TEXT PRIMARY KEY,
  provider_kind TEXT NOT NULL,
  mode TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS provider_sessions (
  scope_key TEXT NOT NULL,
  provider_kind TEXT NOT NULL,
  provider_session_id TEXT NOT NULL,
  last_used_at TEXT NOT NULL,
  metadata_json TEXT,
  PRIMARY KEY (scope_key, provider_kind)
);
