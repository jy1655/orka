use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use tracing::{debug, info, warn};

use crate::model::{
    normalize_scope_key, normalize_session_id, render_help_text, Command, InboundEvent,
    OutboundAction, ProviderKind, ProviderStatus, RuntimeInvokeRequest, RuntimeLogContext,
    RuntimeMode, RuntimePreference,
};
use crate::policy::AccessPolicy;
use crate::ports::{AgentRuntime, EventStore, OutboundSender};
use crate::session::session_key_for_event;

#[derive(Debug, Clone)]
pub struct ProviderRequestMetric {
    pub provider: ProviderKind,
    pub mode: RuntimeMode,
    pub status: ProviderStatus,
    pub total: u64,
}

#[derive(Debug, Clone, Default)]
pub struct GatewayMetricsSnapshot {
    pub inbound_total: u64,
    pub outbound_total: u64,
    pub error_total: u64,
    pub provider_requests: Vec<ProviderRequestMetric>,
}

#[derive(Debug, Default)]
struct GatewayMetrics {
    inbound_total: u64,
    outbound_total: u64,
    error_total: u64,
    provider_requests: HashMap<(ProviderKind, RuntimeMode, ProviderStatus), u64>,
}

pub struct GatewayPipeline {
    store: Arc<dyn EventStore>,
    runtime: Arc<dyn AgentRuntime>,
    outbound: Arc<dyn OutboundSender>,
    policy: AccessPolicy,
    default_runtime: RuntimePreference,
    session_fail_fallback_event: bool,
    operator_env_report: Arc<str>,
    metrics: Mutex<GatewayMetrics>,
}

impl GatewayPipeline {
    pub fn new(
        store: Arc<dyn EventStore>,
        runtime: Arc<dyn AgentRuntime>,
        outbound: Arc<dyn OutboundSender>,
        policy: AccessPolicy,
        default_runtime: RuntimePreference,
        session_fail_fallback_event: bool,
        operator_env_report: String,
    ) -> Self {
        Self {
            store,
            runtime,
            outbound,
            policy,
            default_runtime,
            session_fail_fallback_event,
            operator_env_report: Arc::<str>::from(operator_env_report),
            metrics: Mutex::new(GatewayMetrics::default()),
        }
    }

    pub fn outbound(&self) -> &Arc<dyn OutboundSender> {
        &self.outbound
    }

    pub async fn dispatch_outbound(
        &self,
        action: &OutboundAction,
        scope_key: Option<&str>,
    ) -> Result<()> {
        self.dispatch(action.clone(), None, scope_key).await
    }

    pub fn metrics_snapshot(&self) -> GatewayMetricsSnapshot {
        let lock = self.metrics.lock();
        let guard = match lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        build_metrics_snapshot(&guard)
    }

    fn with_metrics_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut GatewayMetrics) -> R,
    {
        let lock = self.metrics.lock();
        let mut guard = match lock {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    pub async fn handle_event(&self, event: InboundEvent) -> Result<()> {
        if self.store.has_seen(&event.idempotency_key).await? {
            debug!("duplicate event skipped: {}", event.idempotency_key);
            return Ok(());
        }
        self.store.save_inbound(&event).await?;
        self.with_metrics_mut(|metrics| {
            metrics.inbound_total += 1;
        });

        let Some(scope_key) = normalize_scope_key(&session_key_for_event(&event)) else {
            warn!(
                channel = event.channel.as_str(),
                chat_id = %event.chat_id,
                "invalid scope key derived from inbound event; skipping"
            );
            return Ok(());
        };
        if event.is_direct_message
            && !self
                .policy
                .is_operator(event.channel, &event.user_id, &event.claims)
        {
            self.dispatch(
                event.reply("direct messages are restricted to allowlisted operators.".to_string()),
                None,
                Some(&scope_key),
            )
            .await?;
            return Ok(());
        }
        if let Some(command) = Command::parse(&event.text) {
            self.handle_command(command, &event, &scope_key).await?;
            return Ok(());
        }

        if self.store.is_paused(&scope_key).await? {
            let paused =
                event.reply("This session is paused. Use /resume (operator only).".to_string());
            self.dispatch(paused, None, Some(&scope_key)).await?;
            return Ok(());
        }

        let preference = self
            .store
            .get_runtime_preference(&scope_key)
            .await?
            .unwrap_or(self.default_runtime);

        let session_id = match preference.mode {
            RuntimeMode::Session => {
                self.current_provider_session(&scope_key, preference.provider)
                    .await?
            }
            RuntimeMode::Event => None,
        };

        let invoke_request = RuntimeInvokeRequest {
            event: event.clone(),
            scope_key: scope_key.clone(),
            provider: preference.provider,
            mode: preference.mode,
            session_id,
        };

        let invoke_started = Instant::now();
        let mut effective_mode = preference.mode;
        let runtime_response = match self.invoke_runtime(invoke_request.clone()).await {
            Ok(response) => response,
            Err(first_err) => {
                let can_retry_event = preference.mode == RuntimeMode::Session
                    && self.session_fail_fallback_event
                    && invoke_request.session_id.is_some();
                if can_retry_event {
                    effective_mode = RuntimeMode::Event;
                    warn!(
                        scope_key = %scope_key,
                        provider = preference.provider.as_str(),
                        "session invoke failed; clearing session and retrying in event mode"
                    );
                    self.store
                        .clear_provider_session_for(&scope_key, preference.provider)
                        .await?;
                    let retry = RuntimeInvokeRequest {
                        mode: RuntimeMode::Event,
                        session_id: None,
                        ..invoke_request
                    };
                    match self.invoke_runtime(retry).await {
                        Ok(response) => response,
                        Err(second_err) => {
                            self.dispatch(
                                event.reply(
                                    "runtime error: request failed. try again or contact operator."
                                        .to_string(),
                                ),
                                Some(RuntimeLogContext {
                                    provider: preference.provider,
                                    mode: effective_mode,
                                    latency_ms: elapsed_ms(invoke_started),
                                    status: ProviderStatus::Error,
                                }),
                                Some(&scope_key),
                            )
                            .await?;
                            self.with_metrics_mut(|metrics| {
                                metrics.error_total += 1;
                            });
                            return Err(second_err);
                        }
                    }
                } else {
                    self.dispatch(
                        event.reply(
                            "runtime error: request failed. try again or contact operator."
                                .to_string(),
                        ),
                        Some(RuntimeLogContext {
                            provider: preference.provider,
                            mode: effective_mode,
                            latency_ms: elapsed_ms(invoke_started),
                            status: ProviderStatus::Error,
                        }),
                        Some(&scope_key),
                    )
                    .await?;
                    self.with_metrics_mut(|metrics| {
                        metrics.error_total += 1;
                    });
                    return Err(first_err);
                }
            }
        };

        if preference.mode == RuntimeMode::Session {
            if let Some(raw_session_id) = runtime_response.session_id.as_deref() {
                if let Some(session_id) = normalize_session_id(raw_session_id) {
                    self.store
                        .set_provider_session(&scope_key, preference.provider, &session_id)
                        .await?;
                } else {
                    warn!(
                        scope_key = %scope_key,
                        provider = preference.provider.as_str(),
                        "runtime returned invalid session id; clearing cached session"
                    );
                    self.store
                        .clear_provider_session_for(&scope_key, preference.provider)
                        .await?;
                }
            }
        }

        if let Some(reply) = runtime_response.text {
            let action = event.reply(reply);
            self.dispatch(
                action,
                Some(RuntimeLogContext {
                    provider: preference.provider,
                    mode: effective_mode,
                    latency_ms: elapsed_ms(invoke_started),
                    status: ProviderStatus::Success,
                }),
                Some(&scope_key),
            )
            .await?;
        }
        Ok(())
    }

    async fn handle_command(
        &self,
        command: Command,
        event: &InboundEvent,
        scope_key: &str,
    ) -> Result<()> {
        match command {
            Command::Help => self.cmd_help(event, scope_key).await?,
            Command::Status => self.cmd_status(event, scope_key).await?,
            Command::NewSession => self.cmd_new_session(event, scope_key).await?,
            Command::EnvVars => self.cmd_envvars(event, scope_key).await?,
            Command::ProviderList => self.cmd_provider_list(event, scope_key).await?,
            Command::ProviderSet(provider) => {
                self.cmd_provider_set(event, scope_key, provider).await?
            }
            Command::ModeSet(mode) => self.cmd_mode_set(event, scope_key, mode).await?,
            Command::SessionReset => self.cmd_session_reset(event, scope_key).await?,
            Command::Pause => self.cmd_pause(event, scope_key).await?,
            Command::Resume => self.cmd_resume(event, scope_key).await?,
            Command::Audit(count) => self.cmd_audit(event, scope_key, count).await?,
        }
        info!("command processed: {:?}", command);
        Ok(())
    }

    async fn cmd_help(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        let is_operator = self
            .policy
            .is_operator(event.channel, &event.user_id, &event.claims);
        self.dispatch(
            event.reply(render_help_text(event.channel, is_operator)),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_status(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        let paused = self.store.is_paused(scope_key).await?;
        let status_text = if paused { "paused" } else { "active" };
        let preference = self.current_runtime_preference(scope_key).await?;
        let session_state = if preference.mode == RuntimeMode::Session {
            if self
                .current_provider_session(scope_key, preference.provider)
                .await?
                .is_some()
            {
                "active"
            } else {
                "none"
            }
        } else {
            "n/a"
        };
        let text = format!(
            "status: {status_text} · scope={scope_key} · provider={} · mode={} · session={session_state}",
            preference.provider.as_str(),
            preference.mode.as_str()
        );
        self.dispatch(event.reply(text), None, Some(scope_key))
            .await
    }

    async fn cmd_new_session(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        self.store.clear_provider_session(scope_key).await?;
        self.dispatch(
            event.reply(format!("new session: {scope_key}")),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_envvars(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        let report = if self.operator_env_report.trim().is_empty() {
            "envvars: unavailable".to_string()
        } else {
            format!("envvars\n{}", self.operator_env_report)
        };
        self.dispatch(event.reply(report), None, Some(scope_key))
            .await
    }

    async fn cmd_provider_list(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        let preference = self.current_runtime_preference(scope_key).await?;
        self.dispatch(
            event.reply(format!(
                "providers: claude,codex,opencode · current={} · mode={}",
                preference.provider.as_str(),
                preference.mode.as_str()
            )),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_provider_set(
        &self,
        event: &InboundEvent,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        let current = self.current_runtime_preference(scope_key).await?;
        self.store
            .set_runtime_preference(
                scope_key,
                &RuntimePreference {
                    provider,
                    mode: current.mode,
                },
            )
            .await?;

        self.dispatch(
            event.reply(format!(
                "provider set: {} · scope={scope_key}",
                provider.as_str()
            )),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_mode_set(
        &self,
        event: &InboundEvent,
        scope_key: &str,
        mode: RuntimeMode,
    ) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        let current = self.current_runtime_preference(scope_key).await?;
        self.store
            .set_runtime_preference(
                scope_key,
                &RuntimePreference {
                    provider: current.provider,
                    mode,
                },
            )
            .await?;

        self.dispatch(
            event.reply(format!("mode set: {} · scope={scope_key}", mode.as_str())),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_session_reset(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        self.store.clear_provider_session(scope_key).await?;
        self.dispatch(
            event.reply(format!("session reset: {scope_key}")),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_pause(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        self.store.set_paused(scope_key, true).await?;
        self.dispatch(
            event.reply(format!("paused: {scope_key}")),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_resume(&self, event: &InboundEvent, scope_key: &str) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        self.store.set_paused(scope_key, false).await?;
        self.dispatch(
            event.reply(format!("resumed: {scope_key}")),
            None,
            Some(scope_key),
        )
        .await
    }

    async fn cmd_audit(&self, event: &InboundEvent, scope_key: &str, count: usize) -> Result<()> {
        if !self.require_operator(event).await? {
            return Ok(());
        }

        let entries = self.store.query_recent_events(scope_key, count).await?;
        if entries.is_empty() {
            self.dispatch(
                event.reply("audit: no recent events".to_string()),
                None,
                Some(scope_key),
            )
            .await?;
            return Ok(());
        }

        let mut lines = Vec::with_capacity(entries.len());
        for entry in &entries {
            let user = entry.user_id.as_deref().unwrap_or("-");
            lines.push(format!(
                "[{}] {} {} user={} {}",
                entry.created_at, entry.direction, entry.channel, user, entry.text
            ));
        }
        let text = format!("audit ({} entries):\n{}", entries.len(), lines.join("\n"));
        self.dispatch(event.reply(text), None, Some(scope_key))
            .await
    }

    async fn require_operator(&self, event: &InboundEvent) -> Result<bool> {
        if self
            .policy
            .is_operator(event.channel, &event.user_id, &event.claims)
        {
            return Ok(true);
        }
        let scope_key = normalize_scope_key(&session_key_for_event(event));
        self.dispatch(
            event.reply("unauthorized: operator only command".to_string()),
            None,
            scope_key.as_deref(),
        )
        .await?;
        Ok(false)
    }

    async fn current_runtime_preference(&self, scope_key: &str) -> Result<RuntimePreference> {
        Ok(self
            .store
            .get_runtime_preference(scope_key)
            .await?
            .unwrap_or(self.default_runtime))
    }

    async fn current_provider_session(
        &self,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<Option<String>> {
        let stored = self.store.get_provider_session(scope_key, provider).await?;
        let Some(stored) = stored else {
            return Ok(None);
        };

        if let Some(valid) = normalize_session_id(&stored) {
            return Ok(Some(valid));
        }

        warn!(
            scope_key = %scope_key,
            provider = provider.as_str(),
            "invalid cached provider session id; clearing"
        );
        self.store
            .clear_provider_session_for(scope_key, provider)
            .await?;
        Ok(None)
    }

    async fn invoke_runtime(
        &self,
        request: RuntimeInvokeRequest,
    ) -> Result<crate::model::RuntimeInvokeResponse> {
        let provider = request.provider;
        let mode = request.mode;
        match self.runtime.invoke(request).await {
            Ok(response) => {
                self.with_metrics_mut(|metrics| {
                    *metrics
                        .provider_requests
                        .entry((provider, mode, ProviderStatus::Success))
                        .or_insert(0) += 1;
                });
                Ok(response)
            }
            Err(err) => {
                self.with_metrics_mut(|metrics| {
                    *metrics
                        .provider_requests
                        .entry((provider, mode, ProviderStatus::Error))
                        .or_insert(0) += 1;
                });
                Err(err)
            }
        }
    }

    async fn dispatch(
        &self,
        action: OutboundAction,
        runtime: Option<RuntimeLogContext>,
        scope_key: Option<&str>,
    ) -> Result<()> {
        self.store
            .save_outbound(&action, runtime, scope_key)
            .await?;
        self.outbound.send(&action).await?;
        self.with_metrics_mut(|metrics| {
            metrics.outbound_total += 1;
        });
        Ok(())
    }
}

fn build_metrics_snapshot(metrics: &GatewayMetrics) -> GatewayMetricsSnapshot {
    let providers = [
        ProviderKind::Claude,
        ProviderKind::Codex,
        ProviderKind::Opencode,
    ];
    let modes = [RuntimeMode::Session, RuntimeMode::Event];
    let statuses = [ProviderStatus::Success, ProviderStatus::Error];

    let mut provider_requests = Vec::new();
    for provider in providers {
        for mode in modes {
            for status in statuses {
                provider_requests.push(ProviderRequestMetric {
                    provider,
                    mode,
                    status,
                    total: metrics
                        .provider_requests
                        .get(&(provider, mode, status))
                        .copied()
                        .unwrap_or(0),
                });
            }
        }
    }

    GatewayMetricsSnapshot {
        inbound_total: metrics.inbound_total,
        outbound_total: metrics.outbound_total,
        error_total: metrics.error_total,
        provider_requests,
    }
}

fn elapsed_ms(started_at: Instant) -> i64 {
    started_at.elapsed().as_millis().min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use anyhow::{anyhow, Result};
    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::Mutex;

    use super::GatewayPipeline;
    use crate::model::{
        AuditEntry, Channel, InboundEvent, OutboundAction, ProviderKind, RuntimeInvokeRequest,
        RuntimeInvokeResponse, RuntimeLogContext, RuntimeMode, RuntimePreference,
    };
    use crate::policy::AccessPolicy;
    use crate::ports::{AgentRuntime, EventStore, OutboundSender};
    use crate::session::session_key_for_event;

    #[derive(Default)]
    struct MemoryStore {
        seen: Mutex<HashSet<String>>,
        paused: Mutex<HashMap<String, bool>>,
        preferences: Mutex<HashMap<String, RuntimePreference>>,
        sessions: Mutex<HashMap<(String, ProviderKind), String>>,
        inbound: Mutex<Vec<InboundEvent>>,
        outbound: Mutex<Vec<OutboundAction>>,
    }

    #[async_trait]
    impl EventStore for MemoryStore {
        async fn has_seen(&self, idempotency_key: &str) -> Result<bool> {
            Ok(self.seen.lock().await.contains(idempotency_key))
        }

        async fn save_inbound(&self, event: &InboundEvent) -> Result<()> {
            self.seen.lock().await.insert(event.idempotency_key.clone());
            self.inbound.lock().await.push(event.clone());
            Ok(())
        }

        async fn save_outbound(
            &self,
            action: &OutboundAction,
            _runtime: Option<RuntimeLogContext>,
            _scope_key: Option<&str>,
        ) -> Result<()> {
            self.outbound.lock().await.push(action.clone());
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

        async fn clear_provider_session(&self, scope_key: &str) -> Result<()> {
            self.sessions
                .lock()
                .await
                .retain(|(key_scope, _), _| key_scope != scope_key);
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

        async fn query_recent_events(
            &self,
            scope_key: &str,
            limit: usize,
        ) -> Result<Vec<AuditEntry>> {
            let inbound = self.inbound.lock().await;
            let entries: Vec<AuditEntry> = inbound
                .iter()
                .filter(|event| session_key_for_event(event) == scope_key)
                .rev()
                .take(limit)
                .map(|event| AuditEntry {
                    direction: "inbound".to_string(),
                    channel: event.channel.as_str().to_string(),
                    chat_id: event.chat_id.clone(),
                    user_id: Some(event.user_id.clone()),
                    text: event.text.clone(),
                    created_at: event.received_at.to_rfc3339(),
                })
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            Ok(entries)
        }
    }

    struct CaptureRuntime {
        generated_session_id: Option<String>,
        last_request: Mutex<Option<RuntimeInvokeRequest>>,
    }

    impl CaptureRuntime {
        fn new(generated_session_id: Option<&str>) -> Self {
            Self {
                generated_session_id: generated_session_id.map(ToOwned::to_owned),
                last_request: Mutex::new(None),
            }
        }
    }

    #[async_trait]
    impl AgentRuntime for CaptureRuntime {
        async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
            let session_id = request
                .session_id
                .clone()
                .or_else(|| self.generated_session_id.clone());
            *self.last_request.lock().await = Some(request);
            Ok(RuntimeInvokeResponse {
                text: Some("ok".to_string()),
                session_id,
            })
        }
    }

    #[derive(Default)]
    struct FlakySessionRuntime {
        calls: Mutex<Vec<RuntimeMode>>,
    }

    #[async_trait]
    impl AgentRuntime for FlakySessionRuntime {
        async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
            self.calls.lock().await.push(request.mode);
            if request.mode == RuntimeMode::Session {
                return Err(anyhow!("session invoke failed"));
            }
            Ok(RuntimeInvokeResponse {
                text: Some("event-fallback-ok".to_string()),
                session_id: None,
            })
        }
    }

    struct AlwaysFailRuntime;

    #[async_trait]
    impl AgentRuntime for AlwaysFailRuntime {
        async fn invoke(&self, _request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
            Err(anyhow!("runtime failed"))
        }
    }

    #[derive(Default)]
    struct CollectingOutbound {
        actions: Mutex<Vec<OutboundAction>>,
    }

    #[async_trait]
    impl OutboundSender for CollectingOutbound {
        async fn send(&self, action: &OutboundAction) -> Result<()> {
            self.actions.lock().await.push(action.clone());
            Ok(())
        }
    }

    fn event_with_text(idempotency_key: &str, user_id: &str, text: &str) -> InboundEvent {
        InboundEvent {
            idempotency_key: idempotency_key.to_string(),
            channel: Channel::Discord,
            chat_id: "chat-1".to_string(),
            user_id: user_id.to_string(),
            text: text.to_string(),
            received_at: Utc::now(),
            is_direct_message: false,
            reply_token: None,
            claims: vec![],
            attachments: vec![],
        }
    }

    fn direct_message_event(idempotency_key: &str, user_id: &str, text: &str) -> InboundEvent {
        InboundEvent {
            is_direct_message: true,
            ..event_with_text(idempotency_key, user_id, text)
        }
    }

    fn event(idempotency_key: &str) -> InboundEvent {
        event_with_text(idempotency_key, "user-1", "hello")
    }

    #[tokio::test]
    async fn event_mode_does_not_store_provider_session() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime.clone(),
            outbound,
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Codex,
                mode: RuntimeMode::Event,
            },
            false,
            String::new(),
        );

        pipeline.handle_event(event("evt-1")).await?;

        let scope_key = session_key_for_event(&event("evt-1"));
        let stored = store
            .get_provider_session(&scope_key, ProviderKind::Codex)
            .await?;
        assert!(stored.is_none());

        let last = runtime
            .last_request
            .lock()
            .await
            .clone()
            .expect("runtime request exists");
        assert_eq!(last.mode, RuntimeMode::Event);
        assert!(last.session_id.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_mode_reuses_existing_provider_session() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime.clone(),
            outbound,
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline.handle_event(event("evt-1")).await?;
        pipeline.handle_event(event("evt-2")).await?;

        let scope_key = session_key_for_event(&event("evt-1"));
        let stored = store
            .get_provider_session(&scope_key, ProviderKind::Claude)
            .await?;
        assert_eq!(stored.as_deref(), Some("generated-session"));

        let last = runtime
            .last_request
            .lock()
            .await
            .clone()
            .expect("runtime request exists");
        assert_eq!(last.mode, RuntimeMode::Session);
        assert_eq!(last.session_id.as_deref(), Some("generated-session"));
        Ok(())
    }

    #[tokio::test]
    async fn session_mode_does_not_reuse_provider_session_across_users_in_same_chat() -> Result<()>
    {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime.clone(),
            outbound,
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(event_with_text("evt-user-1", "user-1", "hello from user 1"))
            .await?;
        pipeline
            .handle_event(event_with_text("evt-user-2", "user-2", "hello from user 2"))
            .await?;

        let last = runtime
            .last_request
            .lock()
            .await
            .clone()
            .expect("runtime request exists");
        assert_eq!(last.mode, RuntimeMode::Session);
        assert!(last.session_id.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn provider_set_requires_operator() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime,
            outbound.clone(),
            AccessPolicy::new(vec!["discord:admin-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(event_with_text("cmd-1", "user-1", "/provider set codex"))
            .await?;

        let scope_key = session_key_for_event(&event("cmd-1"));
        let preference = store.get_runtime_preference(&scope_key).await?;
        assert!(preference.is_none());

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].text,
            "unauthorized: operator only command".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn provider_set_updates_preference_for_operator() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime,
            outbound.clone(),
            AccessPolicy::new(vec!["discord:user-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(event_with_text("cmd-2", "user-1", "/provider set codex"))
            .await?;

        let scope_key = session_key_for_event(&event("cmd-2"));
        let preference = store
            .get_runtime_preference(&scope_key)
            .await?
            .expect("runtime preference set");
        assert_eq!(preference.provider, ProviderKind::Codex);
        assert_eq!(preference.mode, RuntimeMode::Session);

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert!(actions[0].text.contains("provider set: codex"));
        Ok(())
    }

    #[tokio::test]
    async fn provider_preference_is_isolated_per_user_in_same_chat() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(None));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime.clone(),
            outbound,
            AccessPolicy::new(vec!["discord:user-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Event,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(event_with_text(
                "cmd-user-1",
                "user-1",
                "/provider set codex",
            ))
            .await?;
        pipeline
            .handle_event(event_with_text("evt-user-2", "user-2", "hello from user 2"))
            .await?;

        let last = runtime
            .last_request
            .lock()
            .await
            .clone()
            .expect("runtime request exists");
        assert_eq!(last.provider, ProviderKind::Claude);
        Ok(())
    }

    #[tokio::test]
    async fn session_reset_clears_sessions_for_scope() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime,
            outbound,
            AccessPolicy::new(vec!["discord:user-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        let scope_key = session_key_for_event(&event("cmd-3"));
        store
            .set_provider_session(&scope_key, ProviderKind::Claude, "sess-claude")
            .await?;
        store
            .set_provider_session(&scope_key, ProviderKind::Codex, "sess-codex")
            .await?;

        pipeline
            .handle_event(event_with_text("cmd-3", "user-1", "/session reset"))
            .await?;

        assert!(store
            .get_provider_session(&scope_key, ProviderKind::Claude)
            .await?
            .is_none());
        assert!(store
            .get_provider_session(&scope_key, ProviderKind::Codex)
            .await?
            .is_none());
        Ok(())
    }

    #[tokio::test]
    async fn new_session_clears_sessions_for_any_user() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime,
            outbound.clone(),
            AccessPolicy::new(Vec::<String>::new(), false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        let scope_key = session_key_for_event(&event("cmd-4"));
        store
            .set_provider_session(&scope_key, ProviderKind::Claude, "sess-claude")
            .await?;
        store
            .set_provider_session(&scope_key, ProviderKind::Codex, "sess-codex")
            .await?;

        pipeline
            .handle_event(event_with_text("cmd-4", "user-1", "/new"))
            .await?;

        assert!(store
            .get_provider_session(&scope_key, ProviderKind::Claude)
            .await?
            .is_none());
        assert!(store
            .get_provider_session(&scope_key, ProviderKind::Codex)
            .await?
            .is_none());

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].text,
            "new session: discord:chat-1:user-1".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn help_command_returns_user_visible_commands() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(None));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime,
            outbound.clone(),
            AccessPolicy::new(Vec::<String>::new(), false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(event_with_text("cmd-help", "user-1", "/help"))
            .await?;

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert!(actions[0].text.contains("/help - show available commands"));
        assert!(actions[0]
            .text
            .contains("/new - start a fresh AI session for your current scope"));
        assert!(!actions[0].text.contains("/session_reset"));
        Ok(())
    }

    #[tokio::test]
    async fn direct_messages_require_allowlisted_operator() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(Some("generated-session")));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime,
            outbound.clone(),
            AccessPolicy::new(vec!["discord:admin-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        pipeline
            .handle_event(direct_message_event("dm-1", "user-1", "hello from dm"))
            .await?;

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].text,
            "direct messages are restricted to allowlisted operators.".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn session_failure_can_fallback_to_event_mode() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(FlakySessionRuntime::default());
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime.clone(),
            outbound.clone(),
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            true,
            String::new(),
        );

        let scope_key = session_key_for_event(&event("evt-fallback"));
        store
            .set_provider_session(&scope_key, ProviderKind::Claude, "stale-session")
            .await?;

        pipeline.handle_event(event("evt-fallback")).await?;

        let calls = runtime.calls.lock().await.clone();
        assert_eq!(calls, vec![RuntimeMode::Session, RuntimeMode::Event]);
        assert!(store
            .get_provider_session(&scope_key, ProviderKind::Claude)
            .await?
            .is_none());
        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].text, "event-fallback-ok".to_string());
        Ok(())
    }

    #[tokio::test]
    async fn runtime_failure_sends_safe_error_message() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(AlwaysFailRuntime);
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime,
            outbound.clone(),
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Codex,
                mode: RuntimeMode::Event,
            },
            false,
            String::new(),
        );

        let result = pipeline.handle_event(event("evt-fail")).await;
        assert!(result.is_err());

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0].text,
            "runtime error: request failed. try again or contact operator.".to_string()
        );
        Ok(())
    }

    #[tokio::test]
    async fn status_command_masks_session_identifier() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(None));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store.clone(),
            runtime,
            outbound.clone(),
            AccessPolicy::new(Vec::<String>::new(), true),
            RuntimePreference {
                provider: ProviderKind::Claude,
                mode: RuntimeMode::Session,
            },
            false,
            String::new(),
        );

        let scope_key = session_key_for_event(&event("evt-status"));
        store
            .set_provider_session(&scope_key, ProviderKind::Claude, "secret-session-id")
            .await?;

        pipeline
            .handle_event(event_with_text("evt-status", "user-1", "/status"))
            .await?;

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert!(actions[0].text.contains("session=active"));
        assert!(!actions[0].text.contains("secret-session-id"));
        Ok(())
    }

    #[tokio::test]
    async fn envvars_command_returns_operator_runtime_summary() -> Result<()> {
        let store = Arc::new(MemoryStore::default());
        let runtime = Arc::new(CaptureRuntime::new(None));
        let outbound = Arc::new(CollectingOutbound::default());
        let pipeline = GatewayPipeline::new(
            store,
            runtime,
            outbound.clone(),
            AccessPolicy::new(vec!["discord:user-1".to_string()], false),
            RuntimePreference {
                provider: ProviderKind::Codex,
                mode: RuntimeMode::Session,
            },
            false,
            "runtime_engine=cli\ndefault_provider=codex".to_string(),
        );

        pipeline
            .handle_event(event_with_text("evt-envvars", "user-1", "/envvars"))
            .await?;

        let actions = outbound.actions.lock().await;
        assert_eq!(actions.len(), 1);
        assert!(actions[0].text.contains("envvars"));
        assert!(actions[0].text.contains("runtime_engine=cli"));
        assert!(actions[0].text.contains("default_provider=codex"));
        Ok(())
    }
}
