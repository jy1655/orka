use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::config::{AppConfig, ProviderCommandConfig, RuntimeEngine};
use crate::envfile::find_path_upwards;
use orka_core::model::{ProviderKind, RuntimeMode};
use orka_storage_sqlite::resolve_runtime_migrations_dir;

pub fn run_status(
    cfg: &AppConfig,
    workspace_root: &Path,
    env_path: Option<&Path>,
    deep: bool,
) -> Result<()> {
    let snapshot = collect_status_snapshot(cfg, workspace_root, env_path);
    println!("{}", render_status_report(&snapshot, deep));
    Ok(())
}

pub fn run_doctor(cfg: &AppConfig, workspace_root: &Path, env_path: Option<&Path>) -> Result<()> {
    let snapshot = collect_status_snapshot(cfg, workspace_root, env_path);
    let checks = build_doctor_checks(&snapshot);
    let mut warnings = 0usize;
    let mut errors = 0usize;

    println!("orka doctor");
    println!("workspace: {}", snapshot.workspace_root.display());
    for check in &checks {
        let prefix = match check.level {
            CheckLevel::Ok => "[ok]",
            CheckLevel::Warn => {
                warnings += 1;
                "[warn]"
            }
            CheckLevel::Error => {
                errors += 1;
                "[error]"
            }
        };
        println!("{prefix} {}: {}", check.label, check.detail);
    }
    println!("summary: {} warning(s), {} error(s)", warnings, errors);

    if errors > 0 {
        bail!("doctor found {errors} blocking issue(s)");
    }
    Ok(())
}

pub fn run_onboard(workspace_root: &Path, force: bool) -> Result<()> {
    let template_path = find_path_upwards(workspace_root, ".env.example").ok_or_else(|| {
        anyhow!(
            "could not find .env.example from {}",
            workspace_root.display()
        )
    })?;
    let target_path = template_path
        .parent()
        .unwrap_or(workspace_root)
        .join(".env");
    if target_path.exists() && !force {
        bail!(
            "{} already exists; rerun with `onboard --force` to overwrite",
            target_path.display()
        );
    }

    let template = fs::read_to_string(&template_path)?;
    let detection = detect_binaries_only();
    let rendered = render_onboard_env(&template, &detection);
    fs::write(&target_path, rendered)?;

    println!("generated {}", target_path.display());
    println!(
        "detected provider binaries: claude={} codex={} opencode={}",
        display_detected(&detection.claude),
        display_detected(&detection.codex),
        display_detected(&detection.opencode)
    );
    println!("next steps:");
    println!("1. edit {} and add bot tokens", target_path.display());
    println!("2. run `cargo run -p orka-app -- doctor`");
    println!("3. run `cargo run -p orka-app`");
    Ok(())
}

#[derive(Debug, Clone)]
struct StatusSnapshot {
    workspace_root: PathBuf,
    env_path: Option<PathBuf>,
    runtime_engine: RuntimeEngine,
    default_provider: ProviderKind,
    default_runtime_mode: RuntimeMode,
    health_bind: String,
    health_parse_ok: bool,
    health_reachable: bool,
    database_url: String,
    database_path: Option<PathBuf>,
    database_exists: bool,
    database_size_bytes: Option<u64>,
    migrations_path: Option<PathBuf>,
    discord_configured: bool,
    telegram_configured: bool,
    open_access: bool,
    allowlist_entries: usize,
    store_full_payloads: bool,
    timeout_ms: u64,
    max_output_bytes: usize,
    claude: ProviderBinaryStatus,
    codex: ProviderBinaryStatus,
    opencode: ProviderBinaryStatus,
}

#[derive(Debug, Clone)]
struct ProviderBinaryStatus {
    name: &'static str,
    configured_bin: String,
    resolved_bin: Option<PathBuf>,
    configured_but_missing: bool,
    event_args: Vec<String>,
    session_args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckLevel {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
struct DoctorCheck {
    level: CheckLevel,
    label: &'static str,
    detail: String,
}

#[derive(Debug, Clone, Default)]
struct OnboardDetection {
    claude: Option<PathBuf>,
    codex: Option<PathBuf>,
    opencode: Option<PathBuf>,
}

fn collect_status_snapshot(
    cfg: &AppConfig,
    workspace_root: &Path,
    env_path: Option<&Path>,
) -> StatusSnapshot {
    let database_path = sqlite_path_from_database_url(&cfg.database_url, workspace_root);
    let (database_exists, database_size_bytes) = match database_path
        .as_ref()
        .and_then(|path| fs::metadata(path).ok())
    {
        Some(metadata) => (true, Some(metadata.len())),
        None => (false, None),
    };

    let claude = resolve_provider_binary(
        "claude",
        &cfg.cli_runtime.claude,
        &["claude", "claude.cmd", "claude.exe"],
    );
    let codex = resolve_provider_binary(
        "codex",
        &cfg.cli_runtime.codex,
        &["codex", "codex.cmd", "codex.exe"],
    );
    let opencode = resolve_provider_binary(
        "opencode",
        &cfg.cli_runtime.opencode,
        &["opencode", "opencode.cmd", "opencode.exe"],
    );

    let health_targets = health_probe_targets(&cfg.health_bind);
    let health_parse_ok = !health_targets.is_empty();
    let health_reachable = health_parse_ok && health_endpoint_reachable(&health_targets);

    StatusSnapshot {
        workspace_root: workspace_root.to_path_buf(),
        env_path: env_path.map(Path::to_path_buf),
        runtime_engine: cfg.runtime_engine,
        default_provider: cfg.default_provider,
        default_runtime_mode: cfg.default_runtime_mode,
        health_bind: cfg.health_bind.clone(),
        health_parse_ok,
        health_reachable,
        database_url: cfg.database_url.clone(),
        database_path,
        database_exists,
        database_size_bytes,
        migrations_path: resolve_runtime_migrations_dir().ok(),
        discord_configured: !cfg.discord_bot_token.trim().is_empty(),
        telegram_configured: !cfg.telegram_bot_token.trim().is_empty(),
        open_access: cfg.open_access,
        allowlist_entries: cfg.allowlist.len(),
        store_full_payloads: cfg.store_full_payloads,
        timeout_ms: cfg.cli_runtime.timeout_ms,
        max_output_bytes: cfg.cli_runtime.max_output_bytes,
        claude,
        codex,
        opencode,
    }
}

fn build_doctor_checks(snapshot: &StatusSnapshot) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    checks.push(match &snapshot.env_path {
        Some(path) => ok_check("env", format!("loaded {}", path.display())),
        None => warn_check(
            "env",
            "no .env file found; defaults/environment only".to_string(),
        ),
    });

    checks.push(
        if snapshot.discord_configured || snapshot.telegram_configured {
            ok_check(
                "channels",
                format!(
                    "discord={} telegram={}",
                    configured_label(snapshot.discord_configured),
                    configured_label(snapshot.telegram_configured)
                ),
            )
        } else {
            error_check(
                "channels",
                "no bot token configured for Discord or Telegram".to_string(),
            )
        },
    );

    checks.push(if snapshot.health_parse_ok {
        ok_check("health", format!("bind target {}", snapshot.health_bind))
    } else {
        error_check(
            "health",
            format!("invalid HEALTH_BIND `{}`", snapshot.health_bind),
        )
    });

    checks.push(match &snapshot.database_path {
        Some(path) if path.parent().map(|parent| parent.exists()).unwrap_or(true) => {
            ok_check("database", format!("sqlite path {}", path.display()))
        }
        Some(path) => warn_check(
            "database",
            format!("parent directory missing for {}", path.display()),
        ),
        None => warn_check(
            "database",
            format!(
                "non-sqlite DATABASE_URL `{}` not inspected",
                snapshot.database_url
            ),
        ),
    });

    checks.push(match &snapshot.migrations_path {
        Some(path) => ok_check("migrations", format!("found {}", path.display())),
        None => error_check("migrations", "migrations directory not found".to_string()),
    });

    checks.push(if snapshot.open_access {
        warn_check(
            "policy",
            "OPEN_ACCESS=true allows all non-empty senders to use the gateway".to_string(),
        )
    } else if snapshot.allowlist_entries == 0 {
        warn_check(
            "policy",
            "ALLOWLIST is empty; operator commands will not be available".to_string(),
        )
    } else {
        ok_check(
            "policy",
            format!(
                "open_access=false allowlist_entries={}",
                snapshot.allowlist_entries
            ),
        )
    });

    match snapshot.runtime_engine {
        RuntimeEngine::Echo => {
            checks.push(warn_check(
                "runtime",
                "RUNTIME_ENGINE=echo; providers will not execute real AI CLIs".to_string(),
            ));
        }
        RuntimeEngine::Cli => {
            let default_provider = snapshot.provider(snapshot.default_provider);
            if default_provider.is_resolved() {
                checks.push(ok_check(
                    "runtime",
                    format!(
                        "default provider {} resolved at {}",
                        snapshot.default_provider.as_str(),
                        default_provider
                            .resolved_bin
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default()
                    ),
                ));
            } else {
                checks.push(error_check(
                    "runtime",
                    format!(
                        "default provider {} is not resolvable",
                        snapshot.default_provider.as_str()
                    ),
                ));
            }
        }
    }

    for provider in [&snapshot.claude, &snapshot.codex, &snapshot.opencode] {
        checks.push(
            match (provider.configured_but_missing, &provider.resolved_bin) {
                (true, _) => error_check(
                    provider.name,
                    format!("configured path not found: {}", provider.configured_bin),
                ),
                (false, Some(path)) => {
                    ok_check(provider.name, format!("resolved {}", path.display()))
                }
                (false, None) if snapshot.runtime_engine == RuntimeEngine::Cli => warn_check(
                    provider.name,
                    "not found on PATH and no explicit *_BIN configured".to_string(),
                ),
                _ => ok_check(provider.name, "unused in echo mode".to_string()),
            },
        );
    }

    checks
}

fn render_status_report(snapshot: &StatusSnapshot, deep: bool) -> String {
    let mut lines = vec![
        "orka status".to_string(),
        format!("workspace: {}", snapshot.workspace_root.display()),
        format!(
            "env: {}",
            snapshot
                .env_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "not found".to_string())
        ),
        format!(
            "runtime: {} · default={}/{} · availability={} · timeout={}ms · max_output={} bytes",
            runtime_engine_label(snapshot.runtime_engine),
            snapshot.default_provider.as_str(),
            snapshot.default_runtime_mode.as_str(),
            snapshot.provider(snapshot.default_provider).mode_hint(),
            snapshot.timeout_ms,
            snapshot.max_output_bytes
        ),
        format!(
            "channels: discord={} telegram={}",
            configured_label(snapshot.discord_configured),
            configured_label(snapshot.telegram_configured)
        ),
        format!(
            "health: {} · listener={}",
            snapshot.health_bind,
            if snapshot.health_reachable {
                "reachable"
            } else {
                "unreachable"
            }
        ),
        format!(
            "database: {} · file={}",
            snapshot.database_url,
            snapshot
                .database_path
                .as_ref()
                .map(|path| {
                    if snapshot.database_exists {
                        let size = snapshot
                            .database_size_bytes
                            .map(|bytes| format!("{bytes} bytes"))
                            .unwrap_or_else(|| "present".to_string());
                        format!("{} ({size})", path.display())
                    } else {
                        format!("{} (missing)", path.display())
                    }
                })
                .unwrap_or_else(|| "not inspected".to_string())
        ),
        format!(
            "migrations: {}",
            snapshot
                .migrations_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "missing".to_string())
        ),
        format!(
            "policy: open_access={} · allowlist={} · store_full_payloads={}",
            snapshot.open_access, snapshot.allowlist_entries, snapshot.store_full_payloads
        ),
        "providers:".to_string(),
    ];

    for provider in [&snapshot.claude, &snapshot.codex, &snapshot.opencode] {
        lines.push(format!(
            "  - {}: {}",
            provider.name,
            provider.concise_status()
        ));
        if deep {
            lines.push(format!(
                "    event_args: {}",
                format_args(&provider.event_args)
            ));
            lines.push(format!(
                "    session_args: {}",
                format_args(&provider.session_args)
            ));
        }
    }

    lines.join("\n")
}

fn resolve_provider_binary(
    name: &'static str,
    cfg: &ProviderCommandConfig,
    candidates: &[&str],
) -> ProviderBinaryStatus {
    let configured = cfg.bin.trim().to_string();
    if !configured.is_empty() {
        let path = PathBuf::from(&configured);
        let path_like = path.is_absolute()
            || configured.contains('/')
            || configured.contains('\\')
            || configured.starts_with('.');

        if path_like && path.exists() {
            return ProviderBinaryStatus {
                name,
                configured_bin: configured,
                resolved_bin: Some(path),
                configured_but_missing: false,
                event_args: cfg.event_args.clone(),
                session_args: cfg.session_args.clone(),
            };
        }
        if !path_like {
            if let Some(found) = find_binary_in_path(&[configured.as_str()]) {
                return ProviderBinaryStatus {
                    name,
                    configured_bin: configured,
                    resolved_bin: Some(found),
                    configured_but_missing: false,
                    event_args: cfg.event_args.clone(),
                    session_args: cfg.session_args.clone(),
                };
            }
        }
        return ProviderBinaryStatus {
            name,
            configured_bin: configured,
            resolved_bin: None,
            configured_but_missing: true,
            event_args: cfg.event_args.clone(),
            session_args: cfg.session_args.clone(),
        };
    }

    ProviderBinaryStatus {
        name,
        configured_bin: String::new(),
        resolved_bin: find_binary_in_path(candidates),
        configured_but_missing: false,
        event_args: cfg.event_args.clone(),
        session_args: cfg.session_args.clone(),
    }
}

fn detect_binaries_only() -> OnboardDetection {
    OnboardDetection {
        claude: find_binary_in_path(&["claude", "claude.cmd", "claude.exe"]),
        codex: find_binary_in_path(&["codex", "codex.cmd", "codex.exe"]),
        opencode: find_binary_in_path(&["opencode", "opencode.cmd", "opencode.exe"]),
    }
}

fn render_onboard_env(template: &str, detection: &OnboardDetection) -> String {
    let default_provider = if detection.claude.is_some() {
        ProviderKind::Claude
    } else if detection.codex.is_some() {
        ProviderKind::Codex
    } else if detection.opencode.is_some() {
        ProviderKind::Opencode
    } else {
        ProviderKind::Claude
    };
    let runtime_engine = if detection.claude.is_some()
        || detection.codex.is_some()
        || detection.opencode.is_some()
    {
        "cli"
    } else {
        "echo"
    };

    let mut output = Vec::new();
    for line in template.lines() {
        output.push(if line.starts_with("DEFAULT_PROVIDER=") {
            format!("DEFAULT_PROVIDER={}", default_provider.as_str())
        } else if line.starts_with("RUNTIME_ENGINE=") {
            format!("RUNTIME_ENGINE={runtime_engine}")
        } else if line.starts_with("CLAUDE_BIN=") {
            format!("CLAUDE_BIN={}", env_path_value(detection.claude.as_deref()))
        } else if line.starts_with("CODEX_BIN=") {
            format!("CODEX_BIN={}", env_path_value(detection.codex.as_deref()))
        } else if line.starts_with("OPENCODE_BIN=") {
            format!(
                "OPENCODE_BIN={}",
                env_path_value(detection.opencode.as_deref())
            )
        } else {
            line.to_string()
        });
    }
    output.push(String::new());
    output.join("\n")
}

fn env_path_value(path: Option<&Path>) -> String {
    path.map(normalize_path_for_env).unwrap_or_default()
}

fn normalize_path_for_env(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn find_binary_in_path(candidates: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in candidates {
            let joined = dir.join(candidate);
            if joined.is_file() {
                return Some(joined);
            }
        }
    }
    None
}

fn health_probe_targets(bind: &str) -> Vec<SocketAddr> {
    let Ok(addrs) = bind.to_socket_addrs() else {
        return Vec::new();
    };

    let mut targets = Vec::new();
    for addr in addrs {
        let normalized = match addr.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
            }
            IpAddr::V6(ip) if ip.is_unspecified() => {
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port())
            }
            _ => addr,
        };
        if !targets.contains(&normalized) {
            targets.push(normalized);
        }
    }
    targets
}

fn sqlite_path_from_database_url(raw: &str, workspace_root: &Path) -> Option<PathBuf> {
    let path = raw.strip_prefix("sqlite://")?;
    if path.is_empty() {
        return None;
    }
    let path = PathBuf::from(path);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(workspace_root.join(path))
    }
}

fn health_endpoint_reachable(targets: &[SocketAddr]) -> bool {
    for addr in targets {
        let Ok(mut stream) = TcpStream::connect_timeout(addr, Duration::from_millis(200)) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));

        if stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .is_err()
        {
            continue;
        }

        let mut buf = [0_u8; 64];
        let Ok(read) = stream.read(&mut buf) else {
            continue;
        };
        if read == 0 {
            continue;
        }
        let response = String::from_utf8_lossy(&buf[..read]);
        if response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200") {
            return true;
        }
    }
    false
}

fn runtime_engine_label(engine: RuntimeEngine) -> &'static str {
    match engine {
        RuntimeEngine::Echo => "echo",
        RuntimeEngine::Cli => "cli",
    }
}

fn configured_label(configured: bool) -> &'static str {
    if configured {
        "configured"
    } else {
        "missing"
    }
}

fn display_detected(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "not found".to_string())
}

fn format_args(args: &[String]) -> String {
    if args.is_empty() {
        "(built-in defaults)".to_string()
    } else {
        args.join(" ")
    }
}

fn ok_check(label: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        level: CheckLevel::Ok,
        label,
        detail,
    }
}

fn warn_check(label: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        level: CheckLevel::Warn,
        label,
        detail,
    }
}

fn error_check(label: &'static str, detail: String) -> DoctorCheck {
    DoctorCheck {
        level: CheckLevel::Error,
        label,
        detail,
    }
}

impl StatusSnapshot {
    fn provider(&self, provider: ProviderKind) -> &ProviderBinaryStatus {
        match provider {
            ProviderKind::Claude => &self.claude,
            ProviderKind::Codex => &self.codex,
            ProviderKind::Opencode => &self.opencode,
        }
    }
}

impl ProviderBinaryStatus {
    fn is_resolved(&self) -> bool {
        self.resolved_bin.is_some() && !self.configured_but_missing
    }

    fn mode_hint(&self) -> &'static str {
        if self.is_resolved() {
            "resolved"
        } else {
            "missing"
        }
    }

    fn concise_status(&self) -> String {
        if self.configured_but_missing {
            return format!("configured but missing ({})", self.configured_bin);
        }
        if let Some(path) = &self.resolved_bin {
            if self.configured_bin.is_empty() {
                format!("resolved on PATH ({})", path.display())
            } else {
                format!("configured ({})", path.display())
            }
        } else if self.configured_bin.is_empty() {
            "not found".to_string()
        } else {
            format!("configured ({})", self.configured_bin)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::config::{AppConfig, CliRuntimeConfig, ProviderCommandConfig, RuntimeEngine};
    use orka_core::model::{ProviderKind, RuntimeMode};

    use super::{
        collect_status_snapshot, health_probe_targets, render_onboard_env, resolve_provider_binary,
        OnboardDetection,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = env::temp_dir().join(format!("orka-ops-{label}-{unique}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            discord_bot_token: "discord-token".to_string(),
            telegram_bot_token: "telegram-token".to_string(),
            database_url: "sqlite://data/orka-gateway.db".to_string(),
            health_bind: "127.0.0.1:8787".to_string(),
            allowlist: vec!["discord:1".to_string()],
            open_access: false,
            default_provider: ProviderKind::Codex,
            default_runtime_mode: RuntimeMode::Session,
            session_fail_fallback_event: false,
            shutdown_drain_timeout_ms: 10_000,
            runtime_engine: RuntimeEngine::Cli,
            cli_runtime: CliRuntimeConfig {
                timeout_ms: 90_000,
                max_output_bytes: 262_144,
                claude: ProviderCommandConfig {
                    bin: String::new(),
                    event_args: vec![],
                    session_args: vec![],
                },
                codex: ProviderCommandConfig {
                    bin: String::new(),
                    event_args: vec![],
                    session_args: vec![],
                },
                opencode: ProviderCommandConfig {
                    bin: String::new(),
                    event_args: vec![],
                    session_args: vec![],
                },
            },
            max_concurrent_tasks: 8,
            rate_limit_window_secs: 60,
            rate_limit_max_requests: 0,
            discord_use_embeds: false,
            store_full_payloads: false,
        }
    }

    #[test]
    fn render_onboard_env_prefers_detected_provider_and_cli_mode() {
        let template = "\
DEFAULT_PROVIDER=claude\n\
RUNTIME_ENGINE=echo\n\
CLAUDE_BIN=\n\
CODEX_BIN=\n\
OPENCODE_BIN=\n";
        let rendered = render_onboard_env(
            template,
            &OnboardDetection {
                claude: None,
                codex: Some(PathBuf::from("/usr/local/bin/codex")),
                opencode: None,
            },
        );

        assert!(rendered.contains("DEFAULT_PROVIDER=codex"));
        assert!(rendered.contains("RUNTIME_ENGINE=cli"));
        assert!(rendered.contains("CODEX_BIN=/usr/local/bin/codex"));
    }

    #[test]
    fn render_onboard_env_falls_back_to_echo_when_no_provider_is_detected() {
        let template = "DEFAULT_PROVIDER=claude\nRUNTIME_ENGINE=echo\n";
        let rendered = render_onboard_env(template, &OnboardDetection::default());
        assert!(rendered.contains("DEFAULT_PROVIDER=claude"));
        assert!(rendered.contains("RUNTIME_ENGINE=echo"));
    }

    #[test]
    fn resolve_provider_binary_treats_command_name_as_path_lookup() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = temp_dir("path-lookup");
        let codex_path = dir.join("codex");
        fs::write(&codex_path, "#!/bin/sh\n").expect("write codex stub");

        let original_path = env::var_os("PATH");
        env::set_var("PATH", dir.as_os_str());

        let status = resolve_provider_binary(
            "codex",
            &ProviderCommandConfig {
                bin: "codex".to_string(),
                event_args: vec![],
                session_args: vec![],
            },
            &["codex", "codex.cmd", "codex.exe"],
        );

        assert_eq!(status.resolved_bin.as_deref(), Some(codex_path.as_path()));
        assert!(!status.configured_but_missing);

        match original_path {
            Some(value) => env::set_var("PATH", value),
            None => env::remove_var("PATH"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn collect_status_snapshot_uses_runtime_migrations_resolution() {
        let cfg = sample_config();
        let snapshot =
            collect_status_snapshot(&cfg, Path::new("/definitely/missing/orka-root"), None);
        assert!(snapshot.migrations_path.is_some());
        assert!(snapshot
            .migrations_path
            .as_ref()
            .is_some_and(|path| path.ends_with("migrations")));
    }

    #[test]
    fn health_probe_targets_rewrite_unspecified_bind_to_loopback() {
        let targets = health_probe_targets("0.0.0.0:8787");
        assert_eq!(targets.len(), 1);
        assert!(targets[0].ip().is_loopback());
        assert_eq!(targets[0].port(), 8787);
    }
}
