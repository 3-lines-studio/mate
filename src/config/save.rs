use super::path::config_path;
use super::types::{Config, TUIConfig};

pub fn save_tui(
    dir: &str,
    tools_expanded: bool,
    show_thinking: bool,
    show_subagent_calls: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut data = load_config_map(dir)?;
    let tui = toml::Value::try_from(TUIConfig {
        tools_expanded,
        show_thinking,
        show_subagent_calls,
    })?;
    data.insert("tui".to_string(), tui);
    write_config_map(dir, &data)
}

pub fn save_config(
    dir: &str,
    config: &Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut data = load_config_map(dir)?;
    let edited = toml::Value::try_from(config)?;
    if let toml::Value::Table(t) = edited {
        for (key, val) in t {
            data.insert(key, val);
        }
    }
    write_config_map(dir, &data)
}

fn load_config_map(dir: &str) -> Result<toml::Table, Box<dyn std::error::Error + Send + Sync>> {
    let cfg_path = config_path(dir);
    match std::fs::read_to_string(&cfg_path) {
        Ok(content) => {
            let table: toml::Table = toml::from_str(&content)?;
            Ok(table)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(Box::new(e)),
    }
}

fn write_config_map(
    dir: &str,
    data: &toml::Table,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cfg_path = config_path(dir);
    let content = toml::to_string(data)?;
    let target = if cfg_path.is_symlink() {
        let link = std::fs::read_link(&cfg_path)?;
        if link.is_absolute() {
            link
        } else {
            cfg_path
                .parent()
                .unwrap_or(std::path::Path::new(dir))
                .join(link)
        }
    } else {
        cfg_path.clone()
    };
    let parent = target.parent().unwrap_or(std::path::Path::new(dir));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    tmp.write_all(content.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    tmp.persist(&target)?;
    Ok(())
}
