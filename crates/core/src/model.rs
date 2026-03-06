use std::str::FromStr;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Discord,
    Telegram,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Telegram => "telegram",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Claude,
    Codex,
    Opencode,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
        }
    }
}

impl FromStr for ProviderKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            "opencode" => Ok(Self::Opencode),
            other => Err(anyhow!("unsupported provider: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeMode {
    Session,
    Event,
}

impl RuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Event => "event",
        }
    }
}

impl FromStr for RuntimeMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "session" => Ok(Self::Session),
            "event" => Ok(Self::Event),
            other => Err(anyhow!("unsupported runtime mode: {other}")),
        }
    }
}

const MAX_SCOPE_KEY_LEN: usize = 256;
const MAX_SCOPE_CHANNEL_LEN: usize = 32;
const MAX_SCOPE_CHAT_LEN: usize = 220;
const MAX_SESSION_ID_LEN: usize = 256;

pub fn normalize_scope_key(raw: &str) -> Option<String> {
    let scope_key = raw.trim();
    if scope_key.is_empty() || scope_key.len() > MAX_SCOPE_KEY_LEN {
        return None;
    }

    let (channel, chat_id) = scope_key.split_once(':')?;
    let channel = channel.trim().to_ascii_lowercase();
    let chat_id = chat_id.trim();
    if !is_valid_scope_channel(&channel) || !is_valid_scope_chat(chat_id) {
        return None;
    }

    Some(format!("{channel}:{chat_id}"))
}

pub fn normalize_session_id(raw: &str) -> Option<String> {
    let session_id = raw.trim();
    if session_id.is_empty() || session_id.len() > MAX_SESSION_ID_LEN {
        return None;
    }
    if !session_id.chars().all(is_valid_id_char) {
        return None;
    }
    Some(session_id.to_string())
}

fn is_valid_scope_channel(channel: &str) -> bool {
    !channel.is_empty()
        && channel.len() <= MAX_SCOPE_CHANNEL_LEN
        && channel
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn is_valid_scope_chat(chat_id: &str) -> bool {
    !chat_id.is_empty()
        && chat_id.len() <= MAX_SCOPE_CHAT_LEN
        && chat_id.chars().all(is_valid_id_char)
}

fn is_valid_id_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':' | '@')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub direction: String,
    pub channel: String,
    pub chat_id: String,
    pub user_id: Option<String>,
    pub text: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePreference {
    pub provider: ProviderKind,
    pub mode: RuntimeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderStatus {
    Success,
    Error,
}

impl ProviderStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeLogContext {
    pub provider: ProviderKind,
    pub mode: RuntimeMode,
    pub latency_ms: i64,
    pub status: ProviderStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInvokeRequest {
    pub event: InboundEvent,
    pub scope_key: String,
    pub provider: ProviderKind,
    pub mode: RuntimeMode,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInvokeResponse {
    pub text: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundEvent {
    pub idempotency_key: String,
    pub channel: Channel,
    pub chat_id: String,
    pub user_id: String,
    pub text: String,
    pub received_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_token: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claims: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentMeta>,
}

impl InboundEvent {
    pub fn reply(&self, text: String) -> OutboundAction {
        OutboundAction {
            channel: self.channel,
            chat_id: self.chat_id.clone(),
            text,
            reply_token: self.reply_token.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundAction {
    pub channel: Channel,
    pub chat_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Status,
    Pause,
    Resume,
    ProviderList,
    ProviderSet(ProviderKind),
    ModeSet(RuntimeMode),
    SessionReset,
    Audit(usize),
}

const MAX_AUDIT_COUNT: usize = 50;

impl Command {
    pub fn parse(text: &str) -> Option<Self> {
        let mut tokens = text.split_whitespace();
        let command = tokens.next()?.to_ascii_lowercase();

        match command.as_str() {
            "/status" if tokens.next().is_none() => Some(Self::Status),
            "/pause" if tokens.next().is_none() => Some(Self::Pause),
            "/resume" if tokens.next().is_none() => Some(Self::Resume),
            "/audit" => {
                let count = match tokens.next() {
                    Some(n) => n.parse::<usize>().ok()?.min(MAX_AUDIT_COUNT).max(1),
                    None => 10,
                };
                if tokens.next().is_some() {
                    return None;
                }
                Some(Self::Audit(count))
            }
            "/provider" => {
                let action = tokens.next()?.to_ascii_lowercase();
                match action.as_str() {
                    "list" if tokens.next().is_none() => Some(Self::ProviderList),
                    "set" => {
                        let provider = tokens.next()?.parse::<ProviderKind>().ok()?;
                        if tokens.next().is_some() {
                            return None;
                        }
                        Some(Self::ProviderSet(provider))
                    }
                    _ => None,
                }
            }
            "/mode" => {
                let action = tokens.next()?.to_ascii_lowercase();
                if action.as_str() != "set" {
                    return None;
                }
                let mode = tokens.next()?.parse::<RuntimeMode>().ok()?;
                if tokens.next().is_some() {
                    return None;
                }
                Some(Self::ModeSet(mode))
            }
            "/session" => {
                let action = tokens.next()?.to_ascii_lowercase();
                if action.as_str() != "reset" || tokens.next().is_some() {
                    return None;
                }
                Some(Self::SessionReset)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_scope_key, normalize_session_id, Command, ProviderKind, RuntimeMode};

    #[test]
    fn parse_legacy_commands() {
        assert_eq!(Command::parse("/status"), Some(Command::Status));
        assert_eq!(Command::parse("/pause"), Some(Command::Pause));
        assert_eq!(Command::parse("/resume"), Some(Command::Resume));
    }

    #[test]
    fn parse_runtime_control_commands() {
        assert_eq!(
            Command::parse("/provider list"),
            Some(Command::ProviderList)
        );
        assert_eq!(
            Command::parse("/provider set codex"),
            Some(Command::ProviderSet(ProviderKind::Codex))
        );
        assert_eq!(
            Command::parse("/mode set event"),
            Some(Command::ModeSet(RuntimeMode::Event))
        );
        assert_eq!(
            Command::parse("/session reset"),
            Some(Command::SessionReset)
        );
    }

    #[test]
    fn reject_invalid_runtime_control_commands() {
        assert_eq!(Command::parse("/provider"), None);
        assert_eq!(Command::parse("/provider set"), None);
        assert_eq!(Command::parse("/provider set unknown"), None);
        assert_eq!(Command::parse("/mode set unknown"), None);
        assert_eq!(Command::parse("/session"), None);
        assert_eq!(Command::parse("/session reset now"), None);
    }

    #[test]
    fn normalize_scope_key_rejects_invalid_input() {
        assert_eq!(
            normalize_scope_key("  Discord:12345 ").as_deref(),
            Some("discord:12345")
        );
        assert!(normalize_scope_key("").is_none());
        assert!(normalize_scope_key("discord").is_none());
        assert!(normalize_scope_key("discord:bad value").is_none());
        assert!(normalize_scope_key("discord:../../etc/passwd").is_none());
    }

    #[test]
    fn normalize_session_id_rejects_invalid_input() {
        assert_eq!(
            normalize_session_id("  thread_abc-123:2  ").as_deref(),
            Some("thread_abc-123:2")
        );
        assert!(normalize_session_id("").is_none());
        assert!(normalize_session_id("bad value").is_none());
        assert!(normalize_session_id("bad/session").is_none());
    }
}
