use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::State,
    http::{header::CONTENT_TYPE, HeaderValue, StatusCode},
    response::IntoResponse,
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
}

impl HealthState {
    pub fn new(pipeline: Arc<GatewayPipeline>) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            pipeline,
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

async fn healthz() -> impl IntoResponse {
    "ok"
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    if state.ready.load(Ordering::Relaxed) {
        (StatusCode::OK, "ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not_ready")
    }
}

async fn metrics(State(state): State<HealthState>) -> impl IntoResponse {
    let snapshot = state.pipeline.metrics_snapshot();
    let body = render_metrics(snapshot);

    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
        )],
        body,
    )
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
    use super::render_metrics;
    use orka_core::model::{ProviderKind, ProviderStatus, RuntimeMode};
    use orka_core::pipeline::{GatewayMetricsSnapshot, ProviderRequestMetric};

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
}
