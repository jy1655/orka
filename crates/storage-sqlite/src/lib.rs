use std::fs;
use std::str::FromStr;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use orka_core::model::{
    AuditEntry, OutboundAction, ProviderKind, RuntimeLogContext, RuntimeMode, RuntimePreference,
};
use orka_core::ports::EventStore;
use orka_core::{model::InboundEvent, session::session_key_for_event};

#[derive(Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        // sqlite://data/foo.db 같은 상대 경로도 기본 부팅에서 바로 열리도록 parent dir을 보장한다.
        if let Some(parent) = options.get_filename().parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }
}

#[async_trait]
impl EventStore for SqliteStore {
    async fn has_seen(&self, idempotency_key: &str) -> Result<bool> {
        let row = sqlx::query("SELECT 1 FROM event_log WHERE idempotency_key = ? LIMIT 1")
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    async fn save_inbound(&self, event: &InboundEvent) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let scope = session_key_for_event(event);
        sqlx::query(
            "INSERT INTO sessions(id, channel, chat_id, status, last_seen_at)
       VALUES(?, ?, ?, 'active', ?)
       ON CONFLICT(id) DO UPDATE SET last_seen_at=excluded.last_seen_at",
        )
        .bind(scope)
        .bind(event.channel.as_str())
        .bind(&event.chat_id)
        .bind(now)
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "INSERT OR IGNORE INTO event_log
       (idempotency_key, channel, direction, chat_id, user_id, payload_text, created_at)
       VALUES (?, ?, 'inbound', ?, ?, ?, ?)",
        )
        .bind(&event.idempotency_key)
        .bind(event.channel.as_str())
        .bind(&event.chat_id)
        .bind(&event.user_id)
        .bind(&event.text)
        .bind(event.received_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_outbound(
        &self,
        action: &OutboundAction,
        runtime: Option<RuntimeLogContext>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO event_log
       (idempotency_key, channel, direction, chat_id, user_id, payload_text, created_at, provider_kind, runtime_mode, provider_latency_ms, provider_status)
       VALUES (NULL, ?, 'outbound', ?, NULL, ?, ?, ?, ?, ?, ?)",
        )
        .bind(action.channel.as_str())
        .bind(&action.chat_id)
        .bind(&action.text)
        .bind(Utc::now().to_rfc3339())
        .bind(runtime.map(|ctx| ctx.provider.as_str().to_string()))
        .bind(runtime.map(|ctx| ctx.mode.as_str().to_string()))
        .bind(runtime.map(|ctx| ctx.latency_ms))
        .bind(runtime.map(|ctx| ctx.status.as_str().to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn is_paused(&self, scope_key: &str) -> Result<bool> {
        let row = sqlx::query("SELECT paused FROM command_state WHERE scope_key = ?")
            .bind(scope_key)
            .fetch_optional(&self.pool)
            .await?;
        let paused = row
            .map(|row| row.try_get::<i64, _>("paused").unwrap_or(0) != 0)
            .unwrap_or(false);
        Ok(paused)
    }

    async fn set_paused(&self, scope_key: &str, paused: bool) -> Result<()> {
        sqlx::query(
            "INSERT INTO command_state(scope_key, paused, updated_at)
       VALUES(?, ?, ?)
       ON CONFLICT(scope_key) DO UPDATE SET paused=excluded.paused, updated_at=excluded.updated_at",
        )
        .bind(scope_key)
        .bind(if paused { 1_i64 } else { 0_i64 })
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_runtime_preference(&self, scope_key: &str) -> Result<Option<RuntimePreference>> {
        let row = sqlx::query(
            "SELECT provider_kind, mode
       FROM provider_preferences
       WHERE scope_key = ?",
        )
        .bind(scope_key)
        .fetch_optional(&self.pool)
        .await?;

        let preference = match row {
            Some(row) => {
                let provider = ProviderKind::from_str(&row.try_get::<String, _>("provider_kind")?)?;
                let mode = RuntimeMode::from_str(&row.try_get::<String, _>("mode")?)?;
                Some(RuntimePreference { provider, mode })
            }
            None => None,
        };

        Ok(preference)
    }

    async fn set_runtime_preference(
        &self,
        scope_key: &str,
        preference: &RuntimePreference,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO provider_preferences(scope_key, provider_kind, mode, updated_at)
       VALUES(?, ?, ?, ?)
       ON CONFLICT(scope_key) DO UPDATE SET
         provider_kind=excluded.provider_kind,
         mode=excluded.mode,
         updated_at=excluded.updated_at",
        )
        .bind(scope_key)
        .bind(preference.provider.as_str())
        .bind(preference.mode.as_str())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_provider_session(
        &self,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT provider_session_id
       FROM provider_sessions
       WHERE scope_key = ? AND provider_kind = ?",
        )
        .bind(scope_key)
        .bind(provider.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|row| row.try_get::<String, _>("provider_session_id").ok()))
    }

    async fn set_provider_session(
        &self,
        scope_key: &str,
        provider: ProviderKind,
        session_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO provider_sessions(scope_key, provider_kind, provider_session_id, last_used_at, metadata_json)
       VALUES(?, ?, ?, ?, NULL)
       ON CONFLICT(scope_key, provider_kind) DO UPDATE SET
         provider_session_id=excluded.provider_session_id,
         last_used_at=excluded.last_used_at",
        )
        .bind(scope_key)
        .bind(provider.as_str())
        .bind(session_id)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn clear_provider_session_for(
        &self,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<()> {
        sqlx::query("DELETE FROM provider_sessions WHERE scope_key = ? AND provider_kind = ?")
            .bind(scope_key)
            .bind(provider.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn clear_provider_session(&self, scope_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM provider_sessions WHERE scope_key = ?")
            .bind(scope_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn query_recent_events(&self, scope_key: &str, limit: usize) -> Result<Vec<AuditEntry>> {
        let rows = sqlx::query(
            "SELECT direction, channel, chat_id, user_id, payload_text, created_at
             FROM event_log
             WHERE chat_id = (SELECT chat_id FROM sessions WHERE id = ? LIMIT 1)
             ORDER BY id DESC
             LIMIT ?",
        )
        .bind(scope_key)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut entries: Vec<AuditEntry> = rows
            .iter()
            .map(|row| AuditEntry {
                direction: row.try_get::<String, _>("direction").unwrap_or_default(),
                channel: row.try_get::<String, _>("channel").unwrap_or_default(),
                chat_id: row.try_get::<String, _>("chat_id").unwrap_or_default(),
                user_id: row.try_get::<Option<String>, _>("user_id").unwrap_or(None),
                text: row.try_get::<String, _>("payload_text").unwrap_or_default(),
                created_at: row.try_get::<String, _>("created_at").unwrap_or_default(),
            })
            .collect();

        entries.reverse();
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use anyhow::Result;
    use chrono::Utc;
    use sqlx::Row;

    use super::SqliteStore;
    use orka_core::model::{
        Channel, OutboundAction, ProviderKind, ProviderStatus, RuntimeLogContext, RuntimeMode,
    };
    use orka_core::ports::EventStore;

    fn temp_db_url(test_name: &str) -> (String, PathBuf) {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "orka-storage-test-{}-{}.db",
            test_name,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        (format!("sqlite://{}", path.display()), path)
    }

    fn cleanup_sqlite_files(path: &Path) {
        let _ = std::fs::remove_file(path);
        let wal = path.with_extension(format!(
            "{}-wal",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("db")
        ));
        let shm = path.with_extension(format!(
            "{}-shm",
            path.extension()
                .and_then(|value| value.to_str())
                .unwrap_or("db")
        ));
        let _ = std::fs::remove_file(wal);
        let _ = std::fs::remove_file(shm);
    }

    #[tokio::test]
    async fn save_outbound_persists_runtime_context_fields() -> Result<()> {
        let (database_url, path) = temp_db_url("runtime-context");
        let store = SqliteStore::connect(&database_url).await?;
        store.migrate().await?;

        store
            .save_outbound(
                &OutboundAction {
                    channel: Channel::Discord,
                    chat_id: "123".to_string(),
                    text: "hello".to_string(),
                    reply_token: None,
                },
                Some(RuntimeLogContext {
                    provider: ProviderKind::Codex,
                    mode: RuntimeMode::Event,
                    latency_ms: 321,
                    status: ProviderStatus::Success,
                }),
            )
            .await?;

        let row = sqlx::query(
            "SELECT provider_kind, runtime_mode, provider_latency_ms, provider_status
             FROM event_log
             ORDER BY id DESC
             LIMIT 1",
        )
        .fetch_one(&store.pool)
        .await?;

        assert_eq!(
            row.try_get::<String, _>("provider_kind")?,
            "codex".to_string()
        );
        assert_eq!(
            row.try_get::<String, _>("runtime_mode")?,
            "event".to_string()
        );
        assert_eq!(row.try_get::<i64, _>("provider_latency_ms")?, 321);
        assert_eq!(
            row.try_get::<String, _>("provider_status")?,
            "success".to_string()
        );

        cleanup_sqlite_files(&path);
        Ok(())
    }

    #[tokio::test]
    async fn clear_provider_session_for_removes_only_target_provider() -> Result<()> {
        let (database_url, path) = temp_db_url("session-clear-for");
        let store = SqliteStore::connect(&database_url).await?;
        store.migrate().await?;

        store
            .set_provider_session("discord:1", ProviderKind::Claude, "claude-session")
            .await?;
        store
            .set_provider_session("discord:1", ProviderKind::Codex, "codex-session")
            .await?;

        store
            .clear_provider_session_for("discord:1", ProviderKind::Claude)
            .await?;

        let claude = store
            .get_provider_session("discord:1", ProviderKind::Claude)
            .await?;
        let codex = store
            .get_provider_session("discord:1", ProviderKind::Codex)
            .await?;
        assert!(claude.is_none());
        assert_eq!(codex.as_deref(), Some("codex-session"));

        cleanup_sqlite_files(&path);
        Ok(())
    }
}
