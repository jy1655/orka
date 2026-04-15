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
const MAX_SCOPE_USER_LEN: usize = 220;
const MAX_SESSION_ID_LEN: usize = 256;

pub fn normalize_scope_key(raw: &str) -> Option<String> {
    let scope_key = raw.trim();
    if scope_key.is_empty() || scope_key.len() > MAX_SCOPE_KEY_LEN {
        return None;
    }

    let mut parts = scope_key.split(':');
    let channel = parts.next()?.trim().to_ascii_lowercase();
    let chat_id = parts.next()?.trim();
    let user_id = parts.next().map(str::trim);
    if parts.next().is_some() {
        return None;
    }

    if !is_valid_scope_channel(&channel) || !is_valid_scope_chat(chat_id) {
        return None;
    }

    match user_id {
        Some(user_id) if is_valid_scope_user(user_id) => {
            Some(format!("{channel}:{chat_id}:{user_id}"))
        }
        Some(_) => None,
        None => Some(format!("{channel}:{chat_id}")),
    }
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

fn is_valid_scope_user(user_id: &str) -> bool {
    !user_id.is_empty()
        && user_id.len() <= MAX_SCOPE_USER_LEN
        && user_id.chars().all(is_valid_id_char)
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
    #[serde(default)]
    pub is_direct_message: bool,
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
    Help,
    Status,
    EnvVars,
    Pause,
    Resume,
    ProviderList,
    ProviderSet(ProviderKind),
    ModeSet(RuntimeMode),
    NewSession,
    SessionReset,
    Audit(usize),
}

const MAX_AUDIT_COUNT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub operator_only: bool,
}

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        description: "show available commands",
        operator_only: false,
    },
    CommandSpec {
        name: "status",
        description: "show current scope, provider, and mode",
        operator_only: false,
    },
    CommandSpec {
        name: "new",
        description: "start a fresh AI session for your current scope",
        operator_only: false,
    },
    CommandSpec {
        name: "audit",
        description: "show the recent audit log for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "provider_list",
        description: "show available providers",
        operator_only: false,
    },
    CommandSpec {
        name: "provider_claude",
        description: "switch your current scope to claude",
        operator_only: true,
    },
    CommandSpec {
        name: "provider_codex",
        description: "switch your current scope to codex",
        operator_only: true,
    },
    CommandSpec {
        name: "provider_opencode",
        description: "switch your current scope to opencode",
        operator_only: true,
    },
    CommandSpec {
        name: "mode_session",
        description: "keep one provider session for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "mode_event",
        description: "use stateless event mode for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "session_reset",
        description: "clear all cached provider sessions for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "pause",
        description: "pause AI replies for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "resume",
        description: "resume AI replies for your current scope",
        operator_only: true,
    },
    CommandSpec {
        name: "envvars",
        description: "show the runtime environment summary",
        operator_only: true,
    },
];

pub fn command_specs() -> &'static [CommandSpec] {
    COMMAND_SPECS
}

pub fn render_help_text(channel: Channel, is_operator: bool) -> String {
    let mut lines = Vec::new();
    lines.push("commands:".to_string());
    if channel == Channel::Discord {
        lines.push("/ask <prompt> - ask without sending a plain message".to_string());
    }

    for spec in command_specs().iter().filter(|spec| !spec.operator_only) {
        lines.push(format!("/{} - {}", spec.name, spec.description));
    }

    if is_operator {
        lines.push(String::new());
        lines.push("operator:".to_string());
        for spec in command_specs().iter().filter(|spec| spec.operator_only) {
            lines.push(format!("/{} - {}", spec.name, spec.description));
        }
    } else {
        lines.push(String::new());
        lines.push("operator commands are available only to allowlisted users.".to_string());
    }

    lines.join("\n")
}

impl Command {
    pub fn parse(text: &str) -> Option<Self> {
        let mut tokens = text.split_whitespace();
        let command = tokens.next()?.to_ascii_lowercase();

        match command.as_str() {
            "/help" if tokens.next().is_none() => Some(Self::Help),
            "/status" if tokens.next().is_none() => Some(Self::Status),
            "/new" if tokens.next().is_none() => Some(Self::NewSession),
            "/provider_list" if tokens.next().is_none() => Some(Self::ProviderList),
            "/provider_claude" if tokens.next().is_none() => {
                Some(Self::ProviderSet(ProviderKind::Claude))
            }
            "/provider_codex" if tokens.next().is_none() => {
                Some(Self::ProviderSet(ProviderKind::Codex))
            }
            "/provider_opencode" if tokens.next().is_none() => {
                Some(Self::ProviderSet(ProviderKind::Opencode))
            }
            "/mode_session" if tokens.next().is_none() => Some(Self::ModeSet(RuntimeMode::Session)),
            "/mode_event" if tokens.next().is_none() => Some(Self::ModeSet(RuntimeMode::Event)),
            "/session_reset" if tokens.next().is_none() => Some(Self::SessionReset),
            "/envvars" if tokens.next().is_none() => Some(Self::EnvVars),
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
    use super::{
        normalize_scope_key, normalize_session_id, render_help_text, Channel, Command,
        ProviderKind, RuntimeMode,
    };

    #[test]
    fn parse_legacy_commands() {
        assert_eq!(Command::parse("/help"), Some(Command::Help));
        assert_eq!(Command::parse("/status"), Some(Command::Status));
        assert_eq!(Command::parse("/new"), Some(Command::NewSession));
        assert_eq!(Command::parse("/envvars"), Some(Command::EnvVars));
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
            Command::parse("/provider_list"),
            Some(Command::ProviderList)
        );
        assert_eq!(
            Command::parse("/provider_codex"),
            Some(Command::ProviderSet(ProviderKind::Codex))
        );
        assert_eq!(
            Command::parse("/mode_session"),
            Some(Command::ModeSet(RuntimeMode::Session))
        );
        assert_eq!(
            Command::parse("/session_reset"),
            Some(Command::SessionReset)
        );
        assert_eq!(Command::parse("/new"), Some(Command::NewSession));
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
        assert_eq!(Command::parse("/help now"), None);
        assert_eq!(Command::parse("/session"), None);
        assert_eq!(Command::parse("/session reset now"), None);
    }

    #[test]
    fn render_help_text_hides_operator_commands_for_regular_users() {
        let help = render_help_text(Channel::Telegram, false);
        assert!(help.contains("/help - show available commands"));
        assert!(help.contains("/new - start a fresh AI session for your current scope"));
        assert!(help.contains("/provider_list - show available providers"));
        assert!(!help.contains("/session_reset"));
    }

    #[test]
    fn render_help_text_includes_operator_commands_for_operators() {
        let help = render_help_text(Channel::Discord, true);
        assert!(help.contains("/ask <prompt> - ask without sending a plain message"));
        assert!(help.contains("/provider_codex - switch your current scope to codex"));
        assert!(help.contains(
            "/session_reset - clear all cached provider sessions for your current scope"
        ));
    }

    #[test]
    fn normalize_scope_key_rejects_invalid_input() {
        assert_eq!(
            normalize_scope_key("  Discord:12345 ").as_deref(),
            Some("discord:12345")
        );
        assert_eq!(
            normalize_scope_key("  Discord:12345:User_01 ").as_deref(),
            Some("discord:12345:User_01")
        );
        assert!(normalize_scope_key("").is_none());
        assert!(normalize_scope_key("discord").is_none());
        assert!(normalize_scope_key("discord:bad value").is_none());
        assert!(normalize_scope_key("discord:../../etc/passwd").is_none());
        assert!(normalize_scope_key("discord:12345:user 01").is_none());
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
