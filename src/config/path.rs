use std::path::PathBuf;

use super::types::*;

pub fn dir_for_env(name: &str, xdg: Option<&str>, home: &str) -> String {
    if let Some(xdg) = xdg {
        let mut p = PathBuf::from(xdg);
        p.push(name);
        p.to_string_lossy().to_string()
    } else {
        let mut p = PathBuf::from(home);
        p.push(".config");
        p.push(name);
        p.to_string_lossy().to_string()
    }
}

pub fn dir_for(name: &str) -> String {
    dir_for_env(
        name,
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        &dirs_home(),
    )
}

pub fn dir() -> String {
    dir_for("mate")
}

fn dirs_home() -> String {
    std::env::var("HOME")
        .or_else(|_| {
            std::env::var("USERPROFILE").or_else(|_| {
                let home = std::env::var("HOMEDRIVE").unwrap_or_default();
                let home_path = std::env::var("HOMEPATH").unwrap_or_default();
                Ok::<String, std::env::VarError>(format!("{}{}", home, home_path))
            })
        })
        .unwrap_or_default()
}

pub(crate) fn config_path(dir: &str) -> PathBuf {
    let mut p = PathBuf::from(dir);
    p.push("config.toml");
    p
}

impl Config {
    pub fn default_for(dir: &str) -> Self {
        let mut session_dir = PathBuf::from(dir);
        session_dir.push("sessions");
        Self {
            providers: vec![],
            models: vec![],
            agent: AgentConfig::default(),
            subagents: vec![],
            session: SessionConfig {
                dir: session_dir.to_string_lossy().to_string(),
            },
            slack: SlackConfig::default(),
            telegram: TelegramConfig::default(),
            services: std::collections::HashMap::new(),
            tui: TUIConfig::default(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default_for(&dir())
    }
}
