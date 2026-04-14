use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};

use orka_core::model::{command_specs, Channel, InboundEvent, OutboundAction};
use orka_core::ports::OutboundSender;
use orka_core::text::{normalize_text, normalize_text_with_fallback};

const POLL_TIMEOUT_SECS: i64 = 30;
const MAX_INBOUND_CHARS: usize = 4_000;
const MAX_OUTBOUND_CHARS: usize = 3_500;
const MAX_BACKOFF_SECS: u64 = 30;

pub struct TelegramAdapter {
    token: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
    client: Client,
}

impl TelegramAdapter {
    pub fn new(token: String, inbound_tx: mpsc::Sender<InboundEvent>) -> Self {
        Self {
            token,
            inbound_tx,
            client: Client::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.token.trim().is_empty()
    }

    pub async fn run(self) -> Result<()> {
        if !self.is_enabled() {
            warn!("telegram adapter disabled: TELEGRAM_BOT_TOKEN is empty");
            return Ok(());
        }

        if let Err(err) = self.register_commands().await {
            warn!("telegram setMyCommands failed: {err}");
        }

        info!("telegram adapter started (polling mode)");
        let mut offset = 0_i64;
        let mut backoff_secs = 1_u64;

        loop {
            match self.poll_once(&mut offset).await {
                Ok(()) => backoff_secs = 1,
                Err(err) => {
                    warn!("telegram polling error: {err}");
                    sleep(Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs.saturating_mul(2)).min(MAX_BACKOFF_SECS);
                }
            }
        }
    }

    async fn poll_once(&self, offset: &mut i64) -> Result<()> {
        let request = GetUpdatesRequest {
            offset: *offset,
            timeout: POLL_TIMEOUT_SECS,
            allowed_updates: vec!["message".to_string()],
        };

        let response = self
            .client
            .post(telegram_method_url(&self.token, "getUpdates"))
            .json(&request)
            .send()
            .await
            .context("telegram getUpdates request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("telegram getUpdates response read failed")?;
        if !status.is_success() {
            bail!("telegram getUpdates failed: status={status}");
        }

        let parsed: TelegramResponse<Vec<TelegramUpdate>> =
            serde_json::from_str(&body).context("telegram getUpdates payload decode failed")?;
        if !parsed.ok {
            let reason = parsed
                .description
                .unwrap_or_else(|| "unknown telegram api error".to_string());
            bail!("telegram getUpdates api error: {reason}");
        }

        for update in parsed.result {
            *offset = (*offset).max(update.update_id + 1);
            if let Some(event) = update_to_inbound(update) {
                if let Err(err) = self.inbound_tx.send(event).await {
                    warn!("telegram inbound dropped: pipeline channel closed ({err})");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn register_commands(&self) -> Result<()> {
        let request = SetMyCommandsRequest {
            commands: telegram_bot_commands(),
        };

        let response = self
            .client
            .post(telegram_method_url(&self.token, "setMyCommands"))
            .json(&request)
            .send()
            .await
            .context("telegram setMyCommands request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("telegram setMyCommands response read failed")?;
        if !status.is_success() {
            bail!("telegram setMyCommands failed: status={status}");
        }

        let parsed: TelegramResponse<serde_json::Value> =
            serde_json::from_str(&body).context("telegram setMyCommands payload decode failed")?;
        if !parsed.ok {
            let reason = parsed
                .description
                .unwrap_or_else(|| "unknown telegram api error".to_string());
            bail!("telegram setMyCommands api error: {reason}");
        }

        info!("registered telegram bot commands");
        Ok(())
    }
}

pub struct TelegramOutbound {
    token: String,
    client: Client,
}

impl TelegramOutbound {
    pub fn new(token: String) -> Self {
        Self {
            token,
            client: Client::new(),
        }
    }
}

#[async_trait]
impl OutboundSender for TelegramOutbound {
    async fn send(&self, action: &OutboundAction) -> Result<()> {
        if action.channel != Channel::Telegram {
            return Ok(());
        }
        if self.token.trim().is_empty() {
            bail!("telegram outbound unavailable: missing TELEGRAM_BOT_TOKEN");
        }

        let chat_id = action.chat_id.trim();
        if chat_id.is_empty() {
            bail!("telegram outbound invalid chat_id");
        }

        let request = SendMessageRequest {
            chat_id: chat_id.to_string(),
            text: normalize_outbound_text(&action.text),
        };

        let response = self
            .client
            .post(telegram_method_url(&self.token, "sendMessage"))
            .json(&request)
            .send()
            .await
            .context("telegram sendMessage request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("telegram sendMessage response read failed")?;
        if !status.is_success() {
            bail!("telegram sendMessage failed: status={status}");
        }

        let parsed: TelegramResponse<serde_json::Value> =
            serde_json::from_str(&body).context("telegram sendMessage payload decode failed")?;
        if !parsed.ok {
            let reason = parsed
                .description
                .unwrap_or_else(|| "unknown telegram api error".to_string());
            bail!("telegram sendMessage api error: {reason}");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
struct GetUpdatesRequest {
    offset: i64,
    timeout: i64,
    allowed_updates: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest {
    chat_id: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct SetMyCommandsRequest {
    commands: Vec<TelegramBotCommand>,
}

#[derive(Debug, Serialize)]
struct TelegramBotCommand {
    command: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: T,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
    #[serde(default)]
    from: Option<TelegramUser>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    #[serde(default)]
    is_bot: bool,
}

fn telegram_method_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{}/{method}", token.trim())
}

fn telegram_bot_commands() -> Vec<TelegramBotCommand> {
    command_specs()
        .iter()
        .map(|spec| TelegramBotCommand {
            command: spec.name.to_string(),
            description: spec.description.to_string(),
        })
        .collect()
}

fn update_to_inbound(update: TelegramUpdate) -> Option<InboundEvent> {
    let message = update.message?;
    let user = message.from?;
    if user.is_bot {
        return None;
    }
    let text = normalize_inbound_text(message.text.as_deref().unwrap_or(""))?;

    Some(InboundEvent {
        idempotency_key: format!("telegram:{}:{}", update.update_id, message.message_id),
        channel: Channel::Telegram,
        chat_id: message.chat.id.to_string(),
        user_id: user.id.to_string(),
        text,
        received_at: Utc::now(),
        is_direct_message: message.chat.kind == "private",
        reply_token: None,
        claims: vec![],
        attachments: vec![],
    })
}

fn normalize_inbound_text(raw: &str) -> Option<String> {
    normalize_text(raw, MAX_INBOUND_CHARS)
}

fn normalize_outbound_text(raw: &str) -> String {
    normalize_text_with_fallback(raw, MAX_OUTBOUND_CHARS, "(empty response)")
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_inbound_text, normalize_outbound_text, update_to_inbound, TelegramChat,
        TelegramMessage, TelegramUpdate, TelegramUser,
    };

    #[test]
    fn normalize_text_behaviour() {
        assert_eq!(
            normalize_inbound_text("  hello telegram "),
            Some("hello telegram".to_string())
        );
        assert_eq!(normalize_inbound_text("   "), None);
        assert_eq!(
            normalize_outbound_text("   "),
            "(empty response)".to_string()
        );
    }

    #[test]
    fn converts_update_to_inbound_event() {
        let update = TelegramUpdate {
            update_id: 10,
            message: Some(TelegramMessage {
                message_id: 22,
                chat: TelegramChat {
                    id: 333,
                    kind: "private".to_string(),
                },
                from: Some(TelegramUser {
                    id: 444,
                    is_bot: false,
                }),
                text: Some("ping".to_string()),
            }),
        };

        let inbound = update_to_inbound(update).expect("valid inbound event");
        assert_eq!(inbound.idempotency_key, "telegram:10:22".to_string());
        assert_eq!(inbound.chat_id, "333".to_string());
        assert_eq!(inbound.user_id, "444".to_string());
        assert_eq!(inbound.text, "ping".to_string());
        assert!(inbound.is_direct_message);
    }

    #[test]
    fn ignores_bot_messages() {
        let update = TelegramUpdate {
            update_id: 1,
            message: Some(TelegramMessage {
                message_id: 2,
                chat: TelegramChat {
                    id: 3,
                    kind: "private".to_string(),
                },
                from: Some(TelegramUser {
                    id: 4,
                    is_bot: true,
                }),
                text: Some("hello".to_string()),
            }),
        };
        assert!(update_to_inbound(update).is_none());
    }
}
