use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::State,
    http::{header::AUTHORIZATION, header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::task::JoinHandle;
use tracing::info;

use orka_core::pipeline::{GatewayMetricsSnapshot, GatewayPipeline};

#[derive(Clone)]
pub struct HealthState {
    pub ready: Arc<AtomicBool>,
    pipeline: Arc<GatewayPipeline>,
    bearer_token: Arc<str>,
}

impl HealthState {
    pub fn new(pipeline: Arc<GatewayPipeline>, bearer_token: String) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            pipeline,
            bearer_token: Arc::<str>::from(bearer_token),
        }
    }
}

pub async fn spawn_health_server(
    bind: String,
    state: HealthState,
) -> Result<JoinHandle<Result<()>>> {
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .with_state(state);

    let addr: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("health server listening on {}", addr);
    Ok(tokio::spawn(async move {
        axum::serve(listener, app).await?;
        Ok(())
    }))
}

async fn healthz(State(state): State<HealthState>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    "ok".into_response()
}

async fn readyz(State(state): State<HealthState>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    if state.ready.load(Ordering::Relaxed) {
        (StatusCode::OK, "ready").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not_ready").into_response()
    }
}

async fn metrics(State(state): State<HealthState>, headers: HeaderMap) -> Response {
    if !is_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            [(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            )],
            "unauthorized".to_string(),
        )
            .into_response();
    }
    let snapshot = state.pipeline.metrics_snapshot();
    let body = render_metrics(snapshot);

    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        )],
        body,
    )
        .into_response()
}

fn is_authorized(state: &HealthState, headers: &HeaderMap) -> bool {
    let token = state.bearer_token.trim();
    if token.is_empty() {
        return true;
    }

    let expected = format!("Bearer {token}");
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
}

fn render_metrics(snapshot: GatewayMetricsSnapshot) -> String {
    let mut body = String::new();
    body.push_str("# TYPE orka_inbound_total counter\n");
    body.push_str(&format!("orka_inbound_total {}\n", snapshot.inbound_total));
    body.push_str("# TYPE orka_outbound_total counter\n");
    body.push_str(&format!(
        "orka_outbound_total {}\n",
        snapshot.outbound_total
    ));
    body.push_str("# TYPE orka_error_total counter\n");
    body.push_str(&format!("orka_error_total {}\n", snapshot.error_total));
    body.push_str("# TYPE orka_provider_requests_total counter\n");
    for sample in snapshot.provider_requests {
        body.push_str(&format!(
            "orka_provider_requests_total{{provider=\"{}\",mode=\"{}\",status=\"{}\"}} {}\n",
            sample.provider.as_str(),
            sample.mode.as_str(),
            sample.status.as_str(),
            sample.total
        ));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::{is_authorized, render_metrics, HealthState};
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
    use orka_core::model::{
        AuditEntry, InboundEvent, OutboundAction, ProviderKind, ProviderStatus, RuntimeLogContext,
        RuntimeMode, RuntimePreference,
    };
    use orka_core::pipeline::GatewayPipeline;
    use orka_core::pipeline::{GatewayMetricsSnapshot, ProviderRequestMetric};
    use orka_core::policy::AccessPolicy;
    use orka_core::ports::{EchoAgentRuntime, EventStore, OutboundSender};

    struct NullEventStore;

    #[async_trait]
    impl EventStore for NullEventStore {
        async fn has_seen(&self, _idempotency_key: &str) -> Result<bool> {
            Ok(false)
        }

        async fn save_inbound(&self, _event: &InboundEvent) -> Result<()> {
            Ok(())
        }

        async fn save_outbound(
            &self,
            _action: &OutboundAction,
            _runtime: Option<RuntimeLogContext>,
            _scope_key: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }

        async fn is_paused(&self, _scope_key: &str) -> Result<bool> {
            Ok(false)
        }

        async fn set_paused(&self, _scope_key: &str, _paused: bool) -> Result<()> {
            Ok(())
        }

        async fn get_runtime_preference(
            &self,
            _scope_key: &str,
        ) -> Result<Option<RuntimePreference>> {
            Ok(None)
        }

        async fn set_runtime_preference(
            &self,
            _scope_key: &str,
            _preference: &RuntimePreference,
        ) -> Result<()> {
            Ok(())
        }

        async fn get_provider_session(
            &self,
            _scope_key: &str,
            _provider: ProviderKind,
        ) -> Result<Option<String>> {
            Ok(None)
        }

        async fn set_provider_session(
            &self,
            _scope_key: &str,
            _provider: ProviderKind,
            _session_id: &str,
        ) -> Result<()> {
            Ok(())
        }

        async fn clear_provider_session_for(
            &self,
            _scope_key: &str,
            _provider: ProviderKind,
        ) -> Result<()> {
            Ok(())
        }

        async fn clear_provider_session(&self, _scope_key: &str) -> Result<()> {
            Ok(())
        }

        async fn query_recent_events(
            &self,
            _scope_key: &str,
            _limit: usize,
        ) -> Result<Vec<AuditEntry>> {
            Ok(Vec::new())
        }
    }

    struct NullOutboundSender;

    #[async_trait]
    impl OutboundSender for NullOutboundSender {
        async fn send(&self, _action: &OutboundAction) -> Result<()> {
            Ok(())
        }
    }

    fn health_state(token: &str) -> HealthState {
        let pipeline = GatewayPipeline::new(
            Arc::new(NullEventStore),
            Arc::new(EchoAgentRuntime),
            Arc::new(NullOutboundSender),
            AccessPolicy::new(Vec::<String>::new(), false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Event,
            },
            false,
            String::new(),
        );
        HealthState::new(Arc::new(pipeline), token.to_string())
    }

    #[test]
    fn render_metrics_outputs_prometheus_samples() {
        let body = render_metrics(GatewayMetricsSnapshot {
            inbound_total: 10,
            outbound_total: 7,
            error_total: 2,
            provider_requests: vec![
                ProviderRequestMetric {
                    provider: ProviderKind::Claude,
                    mode: RuntimeMode::Session,
                    status: ProviderStatus::Success,
                    total: 5,
                },
                ProviderRequestMetric {
                    provider: ProviderKind::Codex,
                    mode: RuntimeMode::Event,
                    status: ProviderStatus::Error,
                    total: 1,
                },
            ],
        });

        assert!(body.contains("orka_inbound_total 10"));
        assert!(body.contains("orka_outbound_total 7"));
        assert!(body.contains("orka_error_total 2"));
        assert!(body.contains(
            "orka_provider_requests_total{provider=\"claude\",mode=\"session\",status=\"success\"} 5"
        ));
        assert!(body.contains(
            "orka_provider_requests_total{provider=\"codex\",mode=\"event\",status=\"error\"} 1"
        ));
    }

    #[test]
    fn empty_health_bearer_token_allows_probe_without_authorization_header() {
        let state = health_state("");
        assert!(is_authorized(&state, &HeaderMap::new()));
    }

    #[test]
    fn configured_health_bearer_token_requires_matching_authorization_header() {
        let state = health_state("secret-token");
        let mut headers = HeaderMap::new();
        assert!(!is_authorized(&state, &headers));

        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );
        assert!(is_authorized(&state, &headers));
    }
}
