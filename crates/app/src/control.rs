use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::info;

use orka_core::model::{
    normalize_scope_key, normalize_session_id, ProviderKind, RuntimeMode, RuntimePreference,
};
use orka_core::ports::EventStore;

#[derive(Clone)]
pub struct ControlState {
    pub store: Arc<dyn EventStore>,
    auth_token: Arc<str>,
    default_runtime: RuntimePreference,
}

impl ControlState {
    pub fn new(
        store: Arc<dyn EventStore>,
        auth_token: String,
        default_runtime: RuntimePreference,
    ) -> Self {
        Self {
            store,
            auth_token: Arc::<str>::from(auth_token),
            default_runtime,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ScopeRequest {
    scope_key: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeProviderRequest {
    scope_key: String,
    provider: ProviderKind,
}

#[derive(Debug, Deserialize)]
struct RuntimeModeRequest {
    scope_key: String,
    mode: RuntimeMode,
}

#[derive(Debug, Serialize)]
struct ScopeStatusResponse {
    scope_key: String,
    paused: bool,
}

#[derive(Debug, Serialize)]
struct RuntimeStatusResponse {
    scope_key: String,
    paused: bool,
    provider: ProviderKind,
    mode: RuntimeMode,
    session_id: Option<String>,
}

type ApiResult<T> = std::result::Result<T, (StatusCode, &'static str)>;

pub async fn spawn_control_server(
    bind: String,
    state: ControlState,
) -> Result<JoinHandle<Result<()>>> {
    let app = control_router(state);

    let addr: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("control api listening on {}", addr);
    Ok(tokio::spawn(async move {
        axum::serve(listener, app).await?;
        Ok(())
    }))
}

fn control_router(state: ControlState) -> Router {
    Router::new()
        .route("/control/v1/session/:scope_key", get(get_scope_status))
        .route("/control/v1/pause", post(set_scope_paused))
        .route("/control/v1/resume", post(set_scope_resumed))
        .route("/control/v1/runtime/:scope_key", get(get_runtime_status))
        .route("/control/v1/runtime/provider", post(set_runtime_provider))
        .route("/control/v1/runtime/mode", post(set_runtime_mode))
        .route(
            "/control/v1/runtime/session/reset",
            post(reset_runtime_session),
        )
        .with_state(state)
}

async fn get_scope_status(
    State(state): State<ControlState>,
    Path(scope_key): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<ScopeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key =
        normalize_scope_key(&scope_key).ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    let paused = state
        .store
        .is_paused(&scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
    Ok(Json(ScopeStatusResponse { scope_key, paused }))
}

async fn set_scope_paused(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(req): Json<ScopeRequest>,
) -> ApiResult<Json<ScopeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key = normalize_scope_key(&req.scope_key)
        .ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    state
        .store
        .set_paused(&scope_key, true)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
    Ok(Json(ScopeStatusResponse {
        scope_key,
        paused: true,
    }))
}

async fn set_scope_resumed(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(req): Json<ScopeRequest>,
) -> ApiResult<Json<ScopeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key = normalize_scope_key(&req.scope_key)
        .ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    state
        .store
        .set_paused(&scope_key, false)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
    Ok(Json(ScopeStatusResponse {
        scope_key,
        paused: false,
    }))
}

async fn get_runtime_status(
    State(state): State<ControlState>,
    Path(scope_key): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<RuntimeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key =
        normalize_scope_key(&scope_key).ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    let status = fetch_runtime_status(&state, &scope_key).await?;
    Ok(Json(status))
}

async fn set_runtime_provider(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(req): Json<RuntimeProviderRequest>,
) -> ApiResult<Json<RuntimeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key = normalize_scope_key(&req.scope_key)
        .ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    let current = state
        .store
        .get_runtime_preference(&scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?
        .unwrap_or(state.default_runtime);
    let updated = RuntimePreference {
        provider: req.provider,
        mode: current.mode,
    };
    state
        .store
        .set_runtime_preference(&scope_key, &updated)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;

    let status = fetch_runtime_status(&state, &scope_key).await?;
    Ok(Json(status))
}

async fn set_runtime_mode(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(req): Json<RuntimeModeRequest>,
) -> ApiResult<Json<RuntimeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key = normalize_scope_key(&req.scope_key)
        .ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    let current = state
        .store
        .get_runtime_preference(&scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?
        .unwrap_or(state.default_runtime);
    let updated = RuntimePreference {
        provider: current.provider,
        mode: req.mode,
    };
    state
        .store
        .set_runtime_preference(&scope_key, &updated)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;

    let status = fetch_runtime_status(&state, &scope_key).await?;
    Ok(Json(status))
}

async fn reset_runtime_session(
    State(state): State<ControlState>,
    headers: HeaderMap,
    Json(req): Json<ScopeRequest>,
) -> ApiResult<Json<RuntimeStatusResponse>> {
    authorize(&headers, state.auth_token.as_ref())?;

    let scope_key = normalize_scope_key(&req.scope_key)
        .ok_or((StatusCode::BAD_REQUEST, "invalid scope_key"))?;
    state
        .store
        .clear_provider_session(&scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;

    let status = fetch_runtime_status(&state, &scope_key).await?;
    Ok(Json(status))
}

async fn fetch_runtime_status(
    state: &ControlState,
    scope_key: &str,
) -> ApiResult<RuntimeStatusResponse> {
    let paused = state
        .store
        .is_paused(scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
    let preference = state
        .store
        .get_runtime_preference(scope_key)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?
        .unwrap_or(state.default_runtime);
    let session_id = if preference.mode == RuntimeMode::Session {
        let stored = state
            .store
            .get_provider_session(scope_key, preference.provider)
            .await
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
        match stored {
            Some(session_id) => {
                if let Some(valid) = normalize_session_id(&session_id) {
                    Some(valid)
                } else {
                    state
                        .store
                        .clear_provider_session_for(scope_key, preference.provider)
                        .await
                        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store error"))?;
                    None
                }
            }
            None => None,
        }
    } else {
        None
    };

    Ok(RuntimeStatusResponse {
        scope_key: scope_key.to_string(),
        paused,
        provider: preference.provider,
        mode: preference.mode,
        session_id,
    })
}

fn authorize(headers: &HeaderMap, auth_token: &str) -> ApiResult<()> {
    if auth_token.trim().is_empty() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "control api disabled"));
    }

    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("");

    if token.is_empty() || token != auth_token {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderValue, Request};
    use serde_json::Value;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use orka_core::model::{
        AuditEntry, InboundEvent, OutboundAction, ProviderKind, RuntimeLogContext, RuntimeMode,
        RuntimePreference,
    };
    use orka_core::ports::EventStore;

    #[derive(Default)]
    struct MockStore {
        seen: Mutex<HashSet<String>>,
        paused: Mutex<HashMap<String, bool>>,
        preferences: Mutex<HashMap<String, RuntimePreference>>,
        sessions: Mutex<HashMap<(String, ProviderKind), String>>,
    }

    #[async_trait]
    impl EventStore for MockStore {
        async fn has_seen(&self, idempotency_key: &str) -> Result<bool> {
            Ok(self.seen.lock().await.contains(idempotency_key))
        }

        async fn save_inbound(&self, event: &InboundEvent) -> Result<()> {
            self.seen.lock().await.insert(event.idempotency_key.clone());
            Ok(())
        }

        async fn save_outbound(
            &self,
            _action: &OutboundAction,
            _runtime: Option<RuntimeLogContext>,
        ) -> Result<()> {
            Ok(())
        }

        async fn is_paused(&self, scope_key: &str) -> Result<bool> {
            Ok(self
                .paused
                .lock()
                .await
                .get(scope_key)
                .copied()
                .unwrap_or(false))
        }

        async fn set_paused(&self, scope_key: &str, paused: bool) -> Result<()> {
            self.paused
                .lock()
                .await
                .insert(scope_key.to_string(), paused);
            Ok(())
        }

        async fn get_runtime_preference(
            &self,
            scope_key: &str,
        ) -> Result<Option<RuntimePreference>> {
            Ok(self.preferences.lock().await.get(scope_key).copied())
        }

        async fn set_runtime_preference(
            &self,
            scope_key: &str,
            preference: &RuntimePreference,
        ) -> Result<()> {
            self.preferences
                .lock()
                .await
                .insert(scope_key.to_string(), *preference);
            Ok(())
        }

        async fn get_provider_session(
            &self,
            scope_key: &str,
            provider: ProviderKind,
        ) -> Result<Option<String>> {
            Ok(self
                .sessions
                .lock()
                .await
                .get(&(scope_key.to_string(), provider))
                .cloned())
        }

        async fn set_provider_session(
            &self,
            scope_key: &str,
            provider: ProviderKind,
            session_id: &str,
        ) -> Result<()> {
            self.sessions
                .lock()
                .await
                .insert((scope_key.to_string(), provider), session_id.to_string());
            Ok(())
        }

        async fn clear_provider_session_for(
            &self,
            scope_key: &str,
            provider: ProviderKind,
        ) -> Result<()> {
            self.sessions
                .lock()
                .await
                .remove(&(scope_key.to_string(), provider));
            Ok(())
        }

        async fn clear_provider_session(&self, scope_key: &str) -> Result<()> {
            self.sessions
                .lock()
                .await
                .retain(|(saved_scope_key, _), _| saved_scope_key != scope_key);
            Ok(())
        }

        async fn query_recent_events(
            &self,
            _scope_key: &str,
            _limit: usize,
        ) -> Result<Vec<AuditEntry>> {
            Ok(vec![])
        }
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("valid auth header"),
        );
        headers
    }

    #[test]
    fn authorize_accepts_matching_token() {
        let headers = bearer_headers("expected");
        assert!(authorize(&headers, "expected").is_ok());
    }

    #[test]
    fn authorize_rejects_missing_or_mismatched_token() {
        let empty = HeaderMap::new();
        assert!(authorize(&empty, "expected").is_err());

        let mismatch = bearer_headers("wrong");
        assert!(authorize(&mismatch, "expected").is_err());
    }

    #[test]
    fn normalize_scope_key_trims_and_limits_length() {
        assert_eq!(
            normalize_scope_key("  discord:123  ").as_deref(),
            Some("discord:123")
        );
        assert!(normalize_scope_key("").is_none());
        assert!(normalize_scope_key("   ").is_none());
        assert!(normalize_scope_key(&"x".repeat(257)).is_none());
    }

    fn default_runtime() -> RuntimePreference {
        RuntimePreference {
            provider: ProviderKind::Claude,
            mode: RuntimeMode::Session,
        }
    }

    #[tokio::test]
    async fn runtime_status_rejects_invalid_scope_key() -> Result<()> {
        let store = Arc::new(MockStore::default());
        let app = control_router(ControlState::new(
            store,
            "token".to_string(),
            default_runtime(),
        ));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/control/v1/runtime/discord:bad%20value")
                    .header(AUTHORIZATION, "Bearer token")
                    .body(Body::empty())?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn runtime_provider_endpoint_updates_store_and_response() -> Result<()> {
        let store = Arc::new(MockStore::default());
        let app = control_router(ControlState::new(
            store.clone(),
            "token".to_string(),
            default_runtime(),
        ));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/control/v1/runtime/provider")
                    .header(AUTHORIZATION, "Bearer token")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&serde_json::json!({
                        "scope_key": "discord:123",
                        "provider": "codex"
                    }))?))?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let payload = to_bytes(response.into_body(), usize::MAX).await?;
        let body: Value = serde_json::from_slice(&payload)?;
        assert_eq!(body.get("provider").and_then(Value::as_str), Some("codex"));
        assert_eq!(body.get("mode").and_then(Value::as_str), Some("session"));

        let preference = store.get_runtime_preference("discord:123").await?;
        assert_eq!(
            preference,
            Some(RuntimePreference {
                provider: ProviderKind::Codex,
                mode: RuntimeMode::Session,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn runtime_status_clears_invalid_cached_session_id() -> Result<()> {
        let store = Arc::new(MockStore::default());
        store
            .set_provider_session("discord:55", ProviderKind::Claude, "bad session")
            .await?;

        let app = control_router(ControlState::new(
            store.clone(),
            "token".to_string(),
            default_runtime(),
        ));
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/control/v1/runtime/discord:55")
                    .header(AUTHORIZATION, "Bearer token")
                    .body(Body::empty())?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let payload = to_bytes(response.into_body(), usize::MAX).await?;
        let body: Value = serde_json::from_slice(&payload)?;
        assert!(body.get("session_id").unwrap_or(&Value::Null).is_null());
        assert!(store
            .get_provider_session("discord:55", ProviderKind::Claude)
            .await?
            .is_none());
        Ok(())
    }
}
