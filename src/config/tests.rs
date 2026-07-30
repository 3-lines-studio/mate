use super::*;
use std::io::Write;

fn write_file(path: &std::path::Path, content: &str) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

#[test]
fn test_save_tui_preserves_existing() {
    let dir = tempfile::TempDir::new().unwrap();
    let mate_dir = dir.path().join("mate");
    std::fs::create_dir_all(&mate_dir).unwrap();
    let cfg_path = mate_dir.join("config.toml");
    write_file(&cfg_path, "[agent]\nmodel = \"keep-me\"\n");

    save_tui(&mate_dir.to_string_lossy(), true, false, true).unwrap();

    let data = std::fs::read_to_string(&cfg_path).unwrap();
    let cfg: toml::Table = toml::from_str(&data).unwrap();
    let tui = cfg["tui"].as_table().unwrap();
    assert!(tui["tools_expanded"].as_bool().unwrap());
    assert!(!tui["show_thinking"].as_bool().unwrap());
    let agent = cfg["agent"].as_table().unwrap();
    assert_eq!(agent["model"].as_str().unwrap(), "keep-me");
}

#[test]
fn test_save_tui_no_existing_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let mate_dir = dir.path().join("mate");
    std::fs::create_dir_all(&mate_dir).unwrap();

    save_tui(&mate_dir.to_string_lossy(), true, true, true).unwrap();

    let cfg_path = mate_dir.join("config.toml");
    let data = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(data.contains("tools_expanded = true"));
}

#[test]
fn test_save_tui_does_not_write_defaults() {
    let dir = tempfile::TempDir::new().unwrap();
    let mate_dir = dir.path().join("mate");
    std::fs::create_dir_all(&mate_dir).unwrap();
    let cfg_path = mate_dir.join("config.toml");
    write_file(&cfg_path, "[agent]\nmodel = \"keep-me\"\n");

    save_tui(&mate_dir.to_string_lossy(), true, false, true).unwrap();

    let data = std::fs::read_to_string(&cfg_path).unwrap();
    let cfg: toml::Table = toml::from_str(&data).unwrap();
    // Only [tui] should be written; no default sections.
    assert!(cfg.contains_key("tui"));
    assert!(!cfg.contains_key("session"));
    assert!(!cfg.contains_key("slack"));
    assert!(!cfg.contains_key("telegram"));
    assert!(!cfg.contains_key("providers"));
    assert!(!cfg.contains_key("models"));
    assert!(!cfg.contains_key("subagents"));
    // Existing [agent] section preserved.
    let agent = cfg["agent"].as_table().unwrap();
    assert_eq!(agent["model"].as_str().unwrap(), "keep-me");
}

#[test]
fn test_save_tui_preserves_key_order() {
    let dir = tempfile::TempDir::new().unwrap();
    let mate_dir = dir.path().join("mate");
    std::fs::create_dir_all(&mate_dir).unwrap();
    let cfg_path = mate_dir.join("config.toml");
    // Write keys in non-alphabetical order.
    write_file(
        &cfg_path,
        "[telegram]\nallowed_users = [123]\n\n[agent]\nmodel = \"keep-me\"\n\n[tui]\nshow_thinking = true\n",
    );

    save_tui(&mate_dir.to_string_lossy(), true, false, true).unwrap();

    let data = std::fs::read_to_string(&cfg_path).unwrap();
    let pos_telegram = data.find("[telegram]").unwrap();
    let pos_agent = data.find("[agent]").unwrap();
    let pos_tui = data.find("[tui]").unwrap();
    // Order should match the original file, not alphabetical.
    assert!(pos_telegram < pos_agent);
    assert!(pos_agent < pos_tui);
}

#[test]
fn test_dir_xdg_env() {
    assert_eq!(
        dir_for_env("mate", Some("/tmp/xdg"), "/tmp/home"),
        "/tmp/xdg/mate"
    );
}

#[test]
fn test_dir_no_xdg() {
    assert_eq!(
        dir_for_env("mate", None, "/tmp/home"),
        "/tmp/home/.config/mate"
    );
}

#[test]
fn test_dir_for_custom_name() {
    assert_eq!(
        dir_for_env("alfred", Some("/tmp/xdg"), "/tmp/home"),
        "/tmp/xdg/alfred"
    );
}

#[test]
fn test_dir_for_different_names_isolate() {
    unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg") };
    let a = dir_for("agent-a");
    let b = dir_for("agent-b");
    assert_ne!(a, b);
    unsafe { std::env::remove_var("XDG_CONFIG_HOME") };
}

#[test]
fn test_default_for_custom_dir() {
    let cfg = Config::default_for("/custom/dir");
    assert_eq!(cfg.session.dir, "/custom/dir/sessions");
}

#[test]
fn test_load_from_custom_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    write_file(&cfg_path, "[agent]\nmodel = \"custom-model\"\n");

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.model, "custom-model");
    assert_eq!(
        cfg.session.dir,
        format!("{}/sessions", dir.path().to_string_lossy())
    );
}

#[test]
fn test_load_from_no_config_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.model, "");
    assert_eq!(cfg.agent.tools, vec!["*".to_string()]);
}

#[test]
fn test_default() {
    let cfg = Config::default();
    assert_eq!(cfg.agent.model, "");
    assert_eq!(cfg.agent.tools, vec!["*".to_string()]);
    assert!(!cfg.session.dir.is_empty());
}

#[test]
fn test_load_empty_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.model, "");
}

#[test]
fn test_load_config_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    write_file(&cfg_path, "[agent]\nmodel = \"loaded\"\n");

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.model, "loaded");
}

#[test]
fn test_agent_tools_default_is_all() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    write_file(&cfg_path, "[agent]\nmodel = \"x\"\n");

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.tools, vec!["*".to_string()]);
}

#[test]
fn test_agent_tools_empty_is_none() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    write_file(&cfg_path, "[agent]\ntools = []\n");

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert!(cfg.agent.tools.is_empty());
}

#[test]
fn test_load_secrets_provider_merge() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[[providers]]
id = "openai"
base_url = "https://api.openai.com/v1"

[[providers]]
id = "anthropic"
base_url = "https://api.anthropic.com/v1"
"#,
    );
    write_file(
        &dir.path().join("secrets.toml"),
        r#"
[providers]
openai = "sk-openai-key"
anthropic = "sk-anthropic-key"
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.providers.len(), 2);
    assert_eq!(cfg.providers[0].api_key, "sk-openai-key");
    assert_eq!(cfg.providers[0].base_url, "https://api.openai.com/v1");
    assert_eq!(cfg.providers[1].api_key, "sk-anthropic-key");
    assert_eq!(cfg.providers[1].base_url, "https://api.anthropic.com/v1");
}

#[test]
fn test_load_secrets_partial_keys() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[[providers]]
id = "openai"
base_url = "https://api.openai.com/v1"

[[providers]]
id = "anthropic"
base_url = "https://api.anthropic.com/v1"
"#,
    );
    write_file(
        &dir.path().join("secrets.toml"),
        r#"
[providers]
openai = "sk-openai-key"
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.providers[0].api_key, "sk-openai-key");
    assert_eq!(cfg.providers[1].api_key, "");
    assert_eq!(cfg.providers[1].base_url, "https://api.anthropic.com/v1");
}

#[test]
fn test_load_config_provider_no_api_key() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[[providers]]
id = "openai"
base_url = "https://api.openai.com/v1"
api_key = "sk-should-be-ignored"

[[models]]
id = "gpt4"
provider = "openai"
name = "gpt-4"
context_window = 128000
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.providers.len(), 1);
    assert_eq!(cfg.providers[0].id, "openai");
    assert_eq!(cfg.providers[0].api_key, "");
    assert_eq!(cfg.models.len(), 1);
    assert_eq!(cfg.models[0].name, "gpt-4");
}

#[test]
fn test_load_invalid_toml() {
    let dir = tempfile::TempDir::new().unwrap();
    write_file(&dir.path().join("config.toml"), "this is not toml {{{");
    let result = load_from(&dir.path().to_string_lossy());
    assert!(result.is_err());
}

#[test]
fn test_load_model_price_fields() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[[providers]]
id = "openai"
base_url = "https://api.openai.com/v1"

[[models]]
id = "gpt-4"
provider = "openai"
name = "gpt-4"
context_window = 128000
input_price = 3.0
cached_input_price = 0.3
output_price = 15.0
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    let m = &cfg.models[0];
    assert_eq!(m.input_price, 3.0);
    assert_eq!(m.cached_input_price, 0.3);
    assert_eq!(m.output_price, 15.0);
}

#[test]
fn test_load_model_price_fields_default_to_zero() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[[providers]]
id = "openai"
base_url = "https://api.openai.com/v1"

[[models]]
id = "gpt-4"
provider = "openai"
name = "gpt-4"
context_window = 128000
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    let m = &cfg.models[0];
    assert_eq!(m.input_price, 0.0);
    assert_eq!(m.cached_input_price, 0.0);
    assert_eq!(m.output_price, 0.0);
}

#[test]
fn test_load_tui_config() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        r#"
[tui]
tools_expanded = true
show_thinking = true
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert!(cfg.tui.tools_expanded);
    assert!(cfg.tui.show_thinking);
}

#[test]
fn test_load_secrets_slack_merge() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("secrets.toml"),
        r#"
[slack]
bot_token = "xoxb-123"
app_token = "xapp-456"
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.slack.bot_token, "xoxb-123");
    assert_eq!(cfg.slack.app_token, "xapp-456");
}

#[test]
fn test_load_secrets_services_merge() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("secrets.toml"),
        r#"
[services.picsel]
connection_string = "postgres://localhost:5432/db"
api_key = "svc-key"
"#,
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    let picsel = cfg.services.get("picsel").unwrap();
    assert!(
        picsel
            .get("connection_string")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("postgres://")
    );
}

#[test]
fn test_load_secrets_does_not_override_config() {
    let dir = tempfile::TempDir::new().unwrap();

    write_file(
        &dir.path().join("config.toml"),
        "[agent]\nmodel = \"from-config\"\n",
    );
    write_file(
        &dir.path().join("secrets.toml"),
        "[agent]\nmodel = \"from-secrets\"\n",
    );

    let cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    assert_eq!(cfg.agent.model, "from-config");
}

#[test]
fn test_save_config_secrets_not_leaked() {
    let dir = tempfile::TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.toml");
    write_file(
        &cfg_path,
        r#"
[agent]
model = "keep-me"

[slack]
bot_token = "xoxb-old"
app_token = "xapp-old"

[telegram]
bot_token = "tele-old"
allowed_users = [123]
"#,
    );

    let mut cfg = load_from(&dir.path().to_string_lossy()).unwrap();
    cfg.slack.bot_token = "SECRET-xoxb".to_string();
    cfg.slack.app_token = "SECRET-xapp".to_string();
    cfg.telegram.bot_token = "SECRET-tele".to_string();

    save_config(&dir.path().to_string_lossy(), &cfg).unwrap();

    let written = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(!written.contains("SECRET"));
    assert!(written.contains("allowed_users"));
    assert!(written.contains("model = \"keep-me\"") || written.contains("model = 'keep-me'"));
}

#[test]
fn test_save_config_empty_file() {
    let dir = tempfile::TempDir::new().unwrap();

    let cfg = Config::default_for(&dir.path().to_string_lossy());
    save_config(&dir.path().to_string_lossy(), &cfg).unwrap();

    let cfg_path = dir.path().join("config.toml");
    let data = std::fs::read_to_string(&cfg_path).unwrap();
    assert!(data.contains("[agent]"));
    assert!(data.contains("tools"));
}

#[test]
fn test_save_config_does_not_leak_service_secrets() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();
    write_file(&p.join("config.toml"), "[agent]\nmodel = \"x\"\n");
    write_file(
        &p.join("secrets.toml"),
        "[services.picsel]\nconnection_string = \"postgres://secret-db\"\napi_key = \"SVC-SECRET-KEY\"\n",
    );

    let cfg = load_from(&p.to_string_lossy()).unwrap();
    save_config(&p.to_string_lossy(), &cfg).unwrap();

    let written = std::fs::read_to_string(p.join("config.toml")).unwrap();
    assert!(!written.contains("secret-db"));
    assert!(!written.contains("SVC-SECRET-KEY"));
}

#[test]
fn test_save_config_atomic_no_temp_left() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();

    write_file(&p.join("config.toml"), "[agent]\nmodel = \"x\"\n");

    let cfg = load_from(&p.to_string_lossy()).unwrap();
    save_config(&p.to_string_lossy(), &cfg).unwrap();

    let entries: Vec<_> = std::fs::read_dir(p).unwrap().collect();
    for entry in entries {
        let name = entry.unwrap().file_name();
        assert!(
            name.to_str().unwrap().starts_with("config.toml"),
            "unexpected file left behind: {:?}",
            name
        );
    }
}

#[cfg(unix)]
#[test]
fn test_save_config_preserves_symlink() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::TempDir::new().unwrap();
    let mate_dir = dir.path().join("mate");
    std::fs::create_dir_all(&mate_dir).unwrap();

    let target = dir.path().join("actual.toml");
    write_file(&target, "[agent]\nmodel = \"x\"\n");
    let cfg_path = mate_dir.join("config.toml");
    symlink(&target, &cfg_path).unwrap();

    let cfg = load_from(&mate_dir.to_string_lossy()).unwrap();
    save_config(&mate_dir.to_string_lossy(), &cfg).unwrap();

    assert!(
        std::fs::symlink_metadata(&cfg_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_link(&cfg_path).unwrap(), target);
    let written = std::fs::read_to_string(&target).unwrap();
    assert!(written.contains("model"));
}

#[test]
fn test_save_config_atomic_replaces_existing() {
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path();

    write_file(&p.join("config.toml"), "[agent]\nmodel = \"old\"\n");

    let mut cfg = load_from(&p.to_string_lossy()).unwrap();
    cfg.agent.model = "new".to_string();
    save_config(&p.to_string_lossy(), &cfg).unwrap();

    let written = std::fs::read_to_string(p.join("config.toml")).unwrap();
    assert!(written.contains("model = \"new\"") || written.contains("model = 'new'"));
    assert!(!written.contains("model = \"old\"") && !written.contains("model = 'old'"));
}
