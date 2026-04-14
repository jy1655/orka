use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

pub fn find_path_upwards(start: &Path, name: &str) -> Option<PathBuf> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };

    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn load_dotenv_upwards(start: &Path) -> Result<Option<PathBuf>> {
    let Some(path) = find_path_upwards(start, ".env") else {
        return Ok(None);
    };
    load_env_file(&path)?;
    Ok(Some(path))
}

fn load_env_file(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)?;
    for line in raw.lines() {
        if let Some((key, value)) = parse_env_line(line) {
            if env::var_os(&key).is_none() {
                env::set_var(key, value);
            }
        }
    }
    Ok(())
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }

    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
    let (key, value) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }

    let mut value = value.trim().to_string();
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        value = value[1..value.len() - 1].to_string();
    } else if let Some((prefix, _)) = value.split_once(" #") {
        value = prefix.trim_end().to_string();
    }

    Some((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{find_path_upwards, load_dotenv_upwards, parse_env_line};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = env::temp_dir().join(format!("orka-{label}-{unique}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn parse_env_line_supports_basic_dotenv_syntax() {
        assert_eq!(
            parse_env_line(r#"export CODEX_BIN="/usr/local/bin/codex""#),
            Some(("CODEX_BIN".to_string(), "/usr/local/bin/codex".to_string()))
        );
        assert_eq!(
            parse_env_line("OPEN_ACCESS=false # keep disabled"),
            Some(("OPEN_ACCESS".to_string(), "false".to_string()))
        );
        assert_eq!(parse_env_line(""), None);
        assert_eq!(parse_env_line("# comment"), None);
    }

    #[test]
    fn load_dotenv_upwards_does_not_override_existing_environment_values() {
        let _guard = env_lock().lock().expect("env lock");
        let root = temp_dir("dotenv");
        let nested = root.join("nested").join("deeper");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(
            root.join(".env"),
            "DEFAULT_PROVIDER=codex\nCODEX_BIN=/tmp/codex\n",
        )
        .expect("write env");

        env::set_var("DEFAULT_PROVIDER", "claude");
        env::remove_var("CODEX_BIN");

        let loaded = load_dotenv_upwards(&nested).expect("load dotenv");
        assert_eq!(loaded.as_deref(), Some(root.join(".env").as_path()));
        assert_eq!(
            env::var("DEFAULT_PROVIDER").expect("provider env"),
            "claude".to_string()
        );
        assert_eq!(
            env::var("CODEX_BIN").expect("codex bin"),
            "/tmp/codex".to_string()
        );
        assert_eq!(
            find_path_upwards(&nested, ".env").as_deref(),
            Some(root.join(".env").as_path())
        );

        env::remove_var("DEFAULT_PROVIDER");
        env::remove_var("CODEX_BIN");
        let _ = fs::remove_dir_all(root);
    }
}
