use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;

use crate::model::{
    AuditEntry, Channel, InboundEvent, OutboundAction, ProviderKind, RuntimeInvokeRequest,
    RuntimeInvokeResponse, RuntimeLogContext, RuntimePreference,
};

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn has_seen(&self, idempotency_key: &str) -> Result<bool>;
    async fn save_inbound(&self, event: &InboundEvent) -> Result<()>;
    async fn save_outbound(
        &self,
        action: &OutboundAction,
        runtime: Option<RuntimeLogContext>,
        scope_key: Option<&str>,
    ) -> Result<()>;
    async fn is_paused(&self, scope_key: &str) -> Result<bool>;
    async fn set_paused(&self, scope_key: &str, paused: bool) -> Result<()>;
    async fn get_runtime_preference(&self, scope_key: &str) -> Result<Option<RuntimePreference>>;
    async fn set_runtime_preference(
        &self,
        scope_key: &str,
        preference: &RuntimePreference,
    ) -> Result<()>;
    async fn get_provider_session(
        &self,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<Option<String>>;
    async fn set_provider_session(
        &self,
        scope_key: &str,
        provider: ProviderKind,
        session_id: &str,
    ) -> Result<()>;
    async fn clear_provider_session_for(
        &self,
        scope_key: &str,
        provider: ProviderKind,
    ) -> Result<()>;
    async fn clear_provider_session(&self, scope_key: &str) -> Result<()>;
    async fn query_recent_events(&self, scope_key: &str, limit: usize) -> Result<Vec<AuditEntry>>;
}

#[async_trait]
pub trait OutboundSender: Send + Sync {
    async fn send(&self, action: &OutboundAction) -> Result<()>;

    async fn send_typing(&self, _channel: Channel, _chat_id: &str) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse>;
}

#[derive(Debug, Default)]
pub struct EchoAgentRuntime;

#[async_trait]
impl AgentRuntime for EchoAgentRuntime {
    async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
        let text = format!(
            "echo({}/{}) {}",
            request.provider.as_str(),
            request.mode.as_str(),
            request.event.text.trim()
        );

        let session_id = match request.session_id {
            Some(existing) => Some(existing),
            None if request.mode == crate::model::RuntimeMode::Session => Some(format!(
                "{}_{}_{}",
                request.provider.as_str(),
                request.scope_key.replace(':', "_"),
                Utc::now().timestamp_millis()
            )),
            None => None,
        };

        Ok(RuntimeInvokeResponse {
            text: Some(text),
            session_id,
        })
    }
}
