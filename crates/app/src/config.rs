use std::env;

use orka_core::model::{ProviderKind, RuntimeMode};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEngine {
    Echo,
    Cli,
}

#[derive(Debug, Clone)]
pub struct ProviderCommandConfig {
    pub bin: String,
    pub event_args: Vec<String>,
    pub session_args: Vec<String>,
}

impl ProviderCommandConfig {
    fn from_env(prefix: &str) -> Self {
        let bin = env::var(format!("{prefix}_BIN")).unwrap_or_default();
        let event_args = parse_args_env(&format!("{prefix}_EVENT_ARGS"));
        let session_args = parse_args_env(&format!("{prefix}_SESSION_ARGS"));

        Self {
            bin: bin.trim().to_string(),
            event_args: event_args.clone(),
            session_args: if session_args.is_empty() {
                event_args
            } else {
                session_args
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliRuntimeConfig {
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub claude: ProviderCommandConfig,
    pub codex: ProviderCommandConfig,
    pub opencode: ProviderCommandConfig,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub discord_bot_token: String,
    pub telegram_bot_token: String,
    pub database_url: String,
    pub health_bind: String,
    pub allowlist: Vec<String>,
    pub open_access: bool,
    pub default_provider: ProviderKind,
    pub default_runtime_mode: RuntimeMode,
    pub session_fail_fallback_event: bool,
    pub shutdown_drain_timeout_ms: u64,
    pub runtime_engine: RuntimeEngine,
    pub cli_runtime: CliRuntimeConfig,
    pub max_concurrent_tasks: usize,
    pub rate_limit_window_secs: u64,
    pub rate_limit_max_requests: usize,
    pub discord_use_embeds: bool,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let discord_bot_token = env::var("DISCORD_BOT_TOKEN").unwrap_or_default();
        let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/orka-gateway.db".to_string());
        let health_bind = env::var("HEALTH_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
        let allowlist = env::var("ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let open_access = parse_bool_env("OPEN_ACCESS");
        let default_provider = parse_provider_env("DEFAULT_PROVIDER", ProviderKind::Claude);
        let default_runtime_mode =
            parse_runtime_mode_env("DEFAULT_RUNTIME_MODE", RuntimeMode::Session);
        let session_fail_fallback_event = parse_bool_env("SESSION_FAIL_FALLBACK_EVENT");
        let shutdown_drain_timeout_ms = parse_u64_env("SHUTDOWN_DRAIN_TIMEOUT_MS", 10_000);
        let runtime_engine = parse_runtime_engine_env("RUNTIME_ENGINE", RuntimeEngine::Echo);
        let cli_runtime = CliRuntimeConfig {
            timeout_ms: parse_u64_env("PROVIDER_TIMEOUT_MS", 90_000),
            max_output_bytes: parse_usize_env("MAX_OUTPUT_BYTES", 262_144),
            claude: ProviderCommandConfig::from_env("CLAUDE"),
            codex: ProviderCommandConfig::from_env("CODEX"),
            opencode: ProviderCommandConfig::from_env("OPENCODE"),
        };

        let max_concurrent_tasks = parse_usize_env("MAX_CONCURRENT_TASKS", 8);
        let rate_limit_window_secs = parse_u64_env("RATE_LIMIT_WINDOW_SECS", 60);
        let rate_limit_max_requests = parse_usize_env("RATE_LIMIT_MAX_REQUESTS", 0);
        let discord_use_embeds = parse_bool_env("DISCORD_USE_EMBEDS");

        Self {
            discord_bot_token,
            telegram_bot_token,
            database_url,
            health_bind,
            allowlist,
            open_access,
            default_provider,
            default_runtime_mode,
            session_fail_fallback_event,
            shutdown_drain_timeout_ms,
            runtime_engine,
            cli_runtime,
            max_concurrent_tasks,
            rate_limit_window_secs,
            rate_limit_max_requests,
            discord_use_embeds,
        }
    }
}

fn parse_bool_env(name: &str) -> bool {
    parse_bool(&env::var(name).unwrap_or_default())
}

fn parse_provider_env(name: &str, default: ProviderKind) -> ProviderKind {
    match env::var(name) {
        Ok(value) => {
            if let Some(provider) = parse_provider_kind(&value) {
                provider
            } else {
                if !value.trim().is_empty() {
                    warn!(
                        env = name,
                        value = %value,
                        "invalid provider value; using default"
                    );
                }
                default
            }
        }
        Err(_) => default,
    }
}

fn parse_runtime_mode_env(name: &str, default: RuntimeMode) -> RuntimeMode {
    match env::var(name) {
        Ok(value) => {
            if let Some(mode) = parse_runtime_mode(&value) {
                mode
            } else {
                if !value.trim().is_empty() {
                    warn!(
                        env = name,
                        value = %value,
                        "invalid runtime mode; using default"
                    );
                }
                default
            }
        }
        Err(_) => default,
    }
}

fn parse_runtime_engine_env(name: &str, default: RuntimeEngine) -> RuntimeEngine {
    match env::var(name) {
        Ok(value) => {
            if let Some(engine) = parse_runtime_engine(&value) {
                engine
            } else {
                if !value.trim().is_empty() {
                    warn!(
                        env = name,
                        value = %value,
                        "invalid runtime engine; using default"
                    );
                }
                default
            }
        }
        Err(_) => default,
    }
}

fn parse_u64_env(name: &str, default: u64) -> u64 {
    match env::var(name) {
        Ok(value) => {
            if let Some(parsed) = parse_positive_u64(&value) {
                parsed
            } else {
                if !value.trim().is_empty() {
                    warn!(
                        env = name,
                        value = %value,
                        "invalid positive integer; using default"
                    );
                }
                default
            }
        }
        Err(_) => default,
    }
}

fn parse_usize_env(name: &str, default: usize) -> usize {
    match env::var(name) {
        Ok(value) => {
            if let Some(parsed) = parse_positive_usize(&value) {
                parsed
            } else {
                if !value.trim().is_empty() {
                    warn!(
                        env = name,
                        value = %value,
                        "invalid positive integer; using default"
                    );
                }
                default
            }
        }
        Err(_) => default,
    }
}

fn parse_args_env(name: &str) -> Vec<String> {
    parse_args(&env::var(name).unwrap_or_default())
}

fn parse_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn parse_provider_kind(raw: &str) -> Option<ProviderKind> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<ProviderKind>().ok()
}

fn parse_runtime_mode(raw: &str) -> Option<RuntimeMode> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed.parse::<RuntimeMode>().ok()
}

fn parse_runtime_engine(raw: &str) -> Option<RuntimeEngine> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "cli" => Some(RuntimeEngine::Cli),
        "echo" => Some(RuntimeEngine::Echo),
        _ => None,
    }
}

fn parse_positive_u64(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok().filter(|value| *value > 0)
}

fn parse_positive_usize(raw: &str) -> Option<usize> {
    raw.trim().parse::<usize>().ok().filter(|value| *value > 0)
}

fn parse_args(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if trimmed.starts_with('[') {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(trimmed) {
            return parsed
                .into_iter()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect();
        }
    }

    trimmed.split_whitespace().map(ToOwned::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        parse_args, parse_bool, parse_positive_u64, parse_positive_usize, parse_provider_kind,
        parse_runtime_engine, parse_runtime_mode, RuntimeEngine,
    };
    use orka_core::model::{ProviderKind, RuntimeMode};

    #[test]
    fn parse_bool_accepts_common_truthy_values() {
        assert!(parse_bool("true"));
        assert!(parse_bool(" YES "));
        assert!(parse_bool("1"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool(""));
    }

    #[test]
    fn parse_provider_and_mode_are_case_insensitive() {
        assert_eq!(parse_provider_kind(" CoDeX "), Some(ProviderKind::Codex));
        assert_eq!(parse_runtime_mode(" EvEnT "), Some(RuntimeMode::Event));
        assert_eq!(parse_provider_kind("unknown"), None);
        assert_eq!(parse_runtime_mode("unknown"), None);
    }

    #[test]
    fn parse_runtime_engine_accepts_known_values() {
        assert_eq!(parse_runtime_engine("echo"), Some(RuntimeEngine::Echo));
        assert_eq!(parse_runtime_engine("CLI"), Some(RuntimeEngine::Cli));
        assert_eq!(parse_runtime_engine(""), None);
        assert_eq!(parse_runtime_engine("other"), None);
    }

    #[test]
    fn parse_positive_numbers_reject_invalid_values() {
        assert_eq!(parse_positive_u64("1000"), Some(1000));
        assert_eq!(parse_positive_u64("0"), None);
        assert_eq!(parse_positive_u64("abc"), None);

        assert_eq!(parse_positive_usize("2048"), Some(2048));
        assert_eq!(parse_positive_usize("-1"), None);
        assert_eq!(parse_positive_usize("0"), None);
    }

    #[test]
    fn parse_args_supports_json_and_whitespace() {
        assert_eq!(
            parse_args(r#"["exec","--json","--skip-git-repo-check"]"#),
            vec![
                "exec".to_string(),
                "--json".to_string(),
                "--skip-git-repo-check".to_string()
            ]
        );
        assert_eq!(
            parse_args("exec --json --skip-git-repo-check"),
            vec![
                "exec".to_string(),
                "--json".to_string(),
                "--skip-git-repo-check".to_string()
            ]
        );
    }
}
