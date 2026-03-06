use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::warn;

use crate::config::{CliRuntimeConfig, ProviderCommandConfig};

use orka_core::model::{
    normalize_session_id, ProviderKind, RuntimeInvokeRequest, RuntimeInvokeResponse, RuntimeMode,
};
use orka_core::ports::AgentRuntime;

pub struct CliAgentRuntime {
    config: CliRuntimeConfig,
}

#[derive(Debug, Clone)]
struct ProviderRuntimeArgs {
    bin: String,
    args: Vec<String>,
}

#[derive(Debug, Default, Clone)]
struct ParsedOutput {
    text: Option<String>,
    session_id: Option<String>,
}

impl CliAgentRuntime {
    pub fn new(config: CliRuntimeConfig) -> Result<Self> {
        if config.timeout_ms == 0 {
            bail!("provider timeout must be > 0");
        }
        if config.max_output_bytes == 0 {
            bail!("max output bytes must be > 0");
        }
        Ok(Self { config })
    }

    fn provider_config(&self, provider: ProviderKind) -> &ProviderCommandConfig {
        match provider {
            ProviderKind::Claude => &self.config.claude,
            ProviderKind::Codex => &self.config.codex,
            ProviderKind::Opencode => &self.config.opencode,
        }
    }

    fn default_bin(provider: ProviderKind) -> &'static str {
        match provider {
            ProviderKind::Claude => "claude",
            ProviderKind::Codex => "codex",
            ProviderKind::Opencode => "opencode",
        }
    }

    fn provider_bin_env_name(provider: ProviderKind) -> &'static str {
        match provider {
            ProviderKind::Claude => "CLAUDE_BIN",
            ProviderKind::Codex => "CODEX_BIN",
            ProviderKind::Opencode => "OPENCODE_BIN",
        }
    }

    fn default_event_args(provider: ProviderKind) -> Vec<String> {
        match provider {
            // Claude docs/CLI: -p + --output-format json for non-interactive output.
            ProviderKind::Claude => vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
            ],
            // Codex docs/CLI: codex exec --json for non-interactive JSONL events.
            ProviderKind::Codex => vec![
                "exec".to_string(),
                "--json".to_string(),
                "--skip-git-repo-check".to_string(),
            ],
            // OpenCode source/CLI: opencode run --format json for raw JSON events.
            ProviderKind::Opencode => vec![
                "run".to_string(),
                "--format".to_string(),
                "json".to_string(),
            ],
        }
    }

    fn default_session_args(provider: ProviderKind) -> Vec<String> {
        Self::default_event_args(provider)
    }

    fn build_runtime_args(&self, request: &RuntimeInvokeRequest) -> Result<ProviderRuntimeArgs> {
        let provider_cfg = self.provider_config(request.provider);
        let bin = if provider_cfg.bin.trim().is_empty() {
            Self::default_bin(request.provider).to_string()
        } else {
            provider_cfg.bin.trim().to_string()
        };

        let mut args = match request.mode {
            RuntimeMode::Event => {
                if provider_cfg.event_args.is_empty() {
                    Self::default_event_args(request.provider)
                } else {
                    provider_cfg.event_args.clone()
                }
            }
            RuntimeMode::Session => {
                if provider_cfg.session_args.is_empty() {
                    Self::default_session_args(request.provider)
                } else {
                    provider_cfg.session_args.clone()
                }
            }
        };

        if request.mode == RuntimeMode::Session {
            if let Some(session_id) = request.session_id.as_deref() {
                let session_id = normalize_session_id(session_id)
                    .ok_or_else(|| anyhow!("invalid provider session id"))?;
                match request.provider {
                    ProviderKind::Claude => {
                        args.push("-r".to_string());
                        args.push(session_id.clone());
                    }
                    ProviderKind::Codex => {
                        args.push("resume".to_string());
                        args.push(session_id.clone());
                    }
                    ProviderKind::Opencode => {
                        args.push("--session".to_string());
                        args.push(session_id);
                    }
                }
            }
        }

        // Prevent user prompt text from being interpreted as CLI flags.
        args.push("--".to_string());
        args.push(request.event.text.clone());
        Ok(ProviderRuntimeArgs { bin, args })
    }

    async fn run_provider(&self, request: &RuntimeInvokeRequest) -> Result<String> {
        let runtime_args = self.build_runtime_args(request)?;
        if runtime_args.bin.is_empty() {
            bail!(
                "provider '{}' binary is not configured (set {})",
                request.provider.as_str(),
                Self::provider_bin_env_name(request.provider)
            );
        }

        let mut command = Command::new(&runtime_args.bin);
        command
            .kill_on_drop(true)
            .args(&runtime_args.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("ORKA_PROVIDER", request.provider.as_str())
            .env("ORKA_MODE", request.mode.as_str())
            .env("ORKA_CHANNEL", request.event.channel.as_str())
            .env("ORKA_CHAT_ID", &request.event.chat_id)
            .env("ORKA_USER_ID", &request.event.user_id)
            .env("ORKA_SCOPE_KEY", &request.scope_key);

        let output = timeout(
            Duration::from_millis(self.config.timeout_ms),
            command
                .spawn()
                .with_context(|| {
                    format!(
                        "failed to spawn '{}' for provider '{}'",
                        runtime_args.bin,
                        request.provider.as_str()
                    )
                })?
                .wait_with_output(),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "provider '{}' timed out after {}ms",
                request.provider.as_str(),
                self.config.timeout_ms
            )
        })?
        .context("provider process failed")?;

        let total_output_bytes = output.stdout.len() + output.stderr.len();
        if total_output_bytes > self.config.max_output_bytes {
            bail!(
                "provider '{}' output too large: {} bytes > {} bytes",
                request.provider.as_str(),
                total_output_bytes,
                self.config.max_output_bytes
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            let snippet = sanitize_for_log(&detail);
            bail!(
                "provider '{}' exited with status {}{}",
                request.provider.as_str(),
                output.status,
                if snippet.is_empty() {
                    "".to_string()
                } else {
                    format!(": {snippet}")
                }
            );
        }

        if !stderr.trim().is_empty() {
            let sanitized_stderr = sanitize_for_log(stderr.trim());
            warn!(
                provider = request.provider.as_str(),
                mode = request.mode.as_str(),
                stderr = %sanitized_stderr,
                "provider emitted stderr output"
            );
        }

        Ok(stdout)
    }

    fn parse_output(&self, provider: ProviderKind, stdout: &str) -> ParsedOutput {
        let json_values = parse_json_lines_or_single(stdout);
        if json_values.is_empty() {
            let fallback = stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            return ParsedOutput {
                text: if fallback.is_empty() {
                    None
                } else {
                    Some(fallback)
                },
                session_id: None,
            };
        }

        match provider {
            ProviderKind::Claude => parse_claude_output(&json_values),
            ProviderKind::Codex => parse_codex_output(&json_values),
            ProviderKind::Opencode => parse_opencode_output(&json_values),
        }
    }
}

#[async_trait]
impl AgentRuntime for CliAgentRuntime {
    async fn invoke(&self, request: RuntimeInvokeRequest) -> Result<RuntimeInvokeResponse> {
        let stdout = self.run_provider(&request).await?;
        let parsed = self.parse_output(request.provider, &stdout);
        let session_id = if request.mode == RuntimeMode::Session {
            parsed.session_id.or(request.session_id)
        } else {
            None
        };

        Ok(RuntimeInvokeResponse {
            text: parsed.text,
            session_id,
        })
    }
}

fn parse_json_lines_or_single(input: &str) -> Vec<JsonValue> {
    let mut values = Vec::new();

    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<JsonValue>(line) {
            values.push(value);
        }
    }

    if values.is_empty() {
        if let Ok(value) = serde_json::from_str::<JsonValue>(input.trim()) {
            values.push(value);
        }
    }

    values
}

fn parse_claude_output(values: &[JsonValue]) -> ParsedOutput {
    let mut session_id: Option<String> = None;
    let mut chunks: Vec<String> = Vec::new();

    for value in values {
        if session_id.is_none() {
            session_id = value
                .get("session_id")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned);
        }

        let typ = value.get("type").and_then(JsonValue::as_str).unwrap_or("");
        match typ {
            "result" => {
                if let Some(result) = value.get("result").and_then(JsonValue::as_str) {
                    if !result.trim().is_empty() {
                        chunks.push(result.trim().to_string());
                    }
                }
            }
            "assistant" => {
                if let Some(array) = value
                    .get("message")
                    .and_then(|v| v.get("content"))
                    .and_then(JsonValue::as_array)
                {
                    for part in array {
                        if part.get("type").and_then(JsonValue::as_str) == Some("text") {
                            if let Some(text) = part.get("text").and_then(JsonValue::as_str) {
                                if !text.trim().is_empty() {
                                    chunks.push(text.trim().to_string());
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    ParsedOutput {
        text: last_non_empty(chunks),
        session_id,
    }
}

fn parse_codex_output(values: &[JsonValue]) -> ParsedOutput {
    let mut session_id: Option<String> = None;
    let mut chunks: Vec<String> = Vec::new();

    for value in values {
        let typ = value.get("type").and_then(JsonValue::as_str).unwrap_or("");
        match typ {
            "thread.started" => {
                if session_id.is_none() {
                    session_id = value
                        .get("thread_id")
                        .and_then(JsonValue::as_str)
                        .map(ToOwned::to_owned);
                }
            }
            "item.completed" | "item.updated" => {
                if let Some(item) = value.get("item") {
                    if item.get("type").and_then(JsonValue::as_str) == Some("agent_message") {
                        if let Some(text) = item.get("text").and_then(JsonValue::as_str) {
                            if !text.trim().is_empty() {
                                chunks.push(text.trim().to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    ParsedOutput {
        text: last_non_empty(chunks),
        session_id,
    }
}

fn parse_opencode_output(values: &[JsonValue]) -> ParsedOutput {
    let mut session_id: Option<String> = None;
    let mut chunks: Vec<String> = Vec::new();

    for value in values {
        if session_id.is_none() {
            session_id = value
                .get("sessionID")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned);
        }

        let typ = value.get("type").and_then(JsonValue::as_str).unwrap_or("");
        if typ == "text" {
            if let Some(text) = value
                .get("part")
                .and_then(|part| part.get("text"))
                .and_then(JsonValue::as_str)
            {
                if !text.trim().is_empty() {
                    chunks.push(text.trim().to_string());
                }
            }
        }
    }

    ParsedOutput {
        text: last_non_empty(chunks),
        session_id,
    }
}

fn last_non_empty(items: Vec<String>) -> Option<String> {
    items
        .into_iter()
        .rev()
        .find(|item| !item.trim().is_empty())
        .map(|item| item.trim().to_string())
}

fn sanitize_for_log(raw: &str) -> String {
    const REDACT_KEYWORDS: &[&str] = &[
        "token",
        "password",
        "secret",
        "api_key",
        "authorization",
        "bearer",
    ];
    const MAX_LINES: usize = 8;
    const MAX_CHARS_PER_LINE: usize = 180;

    raw.lines()
        .take(MAX_LINES)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let lowered = line.to_ascii_lowercase();
            if REDACT_KEYWORDS.iter().any(|key| lowered.contains(key)) {
                "[redacted-sensitive-line]".to_string()
            } else {
                line.chars().take(MAX_CHARS_PER_LINE).collect()
            }
        })
        .collect::<Vec<String>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use anyhow::Result;
    use chrono::Utc;

    use super::*;
    use orka_core::model::{Channel, InboundEvent};

    fn echo_config() -> CliRuntimeConfig {
        let echo = ProviderCommandConfig {
            bin: "/bin/echo".to_string(),
            event_args: Vec::new(),
            session_args: Vec::new(),
        };
        CliRuntimeConfig {
            timeout_ms: 10_000,
            max_output_bytes: 64 * 1024,
            claude: echo.clone(),
            codex: echo.clone(),
            opencode: echo,
        }
    }

    fn request(provider: ProviderKind, mode: RuntimeMode) -> RuntimeInvokeRequest {
        RuntimeInvokeRequest {
            event: InboundEvent {
                idempotency_key: "evt-1".to_string(),
                channel: Channel::Discord,
                chat_id: "c1".to_string(),
                user_id: "u1".to_string(),
                text: "hello-runtime".to_string(),
                received_at: Utc::now(),
                reply_token: None,
                claims: vec![],
                attachments: vec![],
            },
            scope_key: "discord:c1".to_string(),
            provider,
            mode,
            session_id: None,
        }
    }

    #[tokio::test]
    async fn cli_runtime_falls_back_to_plain_text_output() -> Result<()> {
        let runtime = CliAgentRuntime::new(echo_config())?;
        let res = runtime
            .invoke(request(ProviderKind::Opencode, RuntimeMode::Event))
            .await?;
        assert_eq!(
            res.text.as_deref(),
            Some("run --format json -- hello-runtime")
        );
        assert!(res.session_id.is_none());
        Ok(())
    }

    #[test]
    fn parse_claude_json_result() {
        let values =
            parse_json_lines_or_single(r#"{"type":"result","result":"hi","session_id":"sid-1"}"#);
        let parsed = parse_claude_output(&values);
        assert_eq!(parsed.text.as_deref(), Some("hi"));
        assert_eq!(parsed.session_id.as_deref(), Some("sid-1"));
    }

    #[test]
    fn parse_codex_json_events() {
        let raw = r#"{"type":"thread.started","thread_id":"thread-1"}
{"type":"item.completed","item":{"type":"agent_message","text":"done"}}"#;
        let values = parse_json_lines_or_single(raw);
        let parsed = parse_codex_output(&values);
        assert_eq!(parsed.text.as_deref(), Some("done"));
        assert_eq!(parsed.session_id.as_deref(), Some("thread-1"));
    }

    #[test]
    fn parse_opencode_json_events() {
        let raw = r#"{"type":"step_start","sessionID":"ses_1","part":{"type":"step-start"}}
{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"hello"}}"#;
        let values = parse_json_lines_or_single(raw);
        let parsed = parse_opencode_output(&values);
        assert_eq!(parsed.text.as_deref(), Some("hello"));
        assert_eq!(parsed.session_id.as_deref(), Some("ses_1"));
    }

    #[tokio::test]
    async fn cli_runtime_returns_error_when_binary_missing() -> Result<()> {
        let mut config = echo_config();
        config.codex.bin = "/definitely/not/found/codex".to_string();
        config.codex.event_args = vec!["exec".to_string(), "--json".to_string()];
        let runtime = CliAgentRuntime::new(config)?;
        let err = runtime
            .invoke(request(ProviderKind::Codex, RuntimeMode::Event))
            .await
            .expect_err("missing provider binary should fail");
        assert!(err.to_string().contains("failed to spawn"));
        Ok(())
    }

    #[cfg(unix)]
    fn write_executable_script(name: &str, body: &str) -> Result<PathBuf> {
        use std::os::unix::fs::PermissionsExt;

        let mut path = std::env::temp_dir();
        path.push(format!(
            "orka-runtime-test-{}-{}.sh",
            name,
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let content = format!("#!/bin/sh\nset -eu\n{body}\n");
        fs::write(&path, content)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_runtime_times_out_provider_process() -> Result<()> {
        let script = write_executable_script("timeout", "sleep 2")?;
        let mut config = echo_config();
        config.codex.bin = script.display().to_string();
        config.codex.event_args = Vec::new();
        config.timeout_ms = 50;
        let runtime = CliAgentRuntime::new(config)?;

        let err = runtime
            .invoke(request(ProviderKind::Codex, RuntimeMode::Event))
            .await
            .expect_err("long-running provider should time out");
        assert!(err.to_string().contains("timed out"));

        let _ = fs::remove_file(script);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_runtime_fails_on_output_size_limit() -> Result<()> {
        let script = write_executable_script(
            "output-limit",
            "i=0; while [ \"$i\" -lt 10000 ]; do printf x; i=$((i+1)); done",
        )?;
        let mut config = echo_config();
        config.codex.bin = script.display().to_string();
        config.codex.event_args = Vec::new();
        config.max_output_bytes = 1024;
        let runtime = CliAgentRuntime::new(config)?;

        let err = runtime
            .invoke(request(ProviderKind::Codex, RuntimeMode::Event))
            .await
            .expect_err("oversized provider output should fail");
        assert!(err.to_string().contains("output too large"));

        let _ = fs::remove_file(script);
        Ok(())
    }
}
