use crate::agent;
use crate::config::Config;
use crate::prompts;
use crate::session::store::Store;
use crate::skills;
use crate::tools;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;

use super::Deps;
use super::resolve::{resolve_client, resolve_subagents};

pub fn init_with_config(
    cfg: Config,
    system_prompt: String,
    cwd: &str,
    store_dir: &str,
    verbose: bool,
) -> Result<Deps, Box<dyn std::error::Error + Send + Sync>> {
    for p in &cfg.providers {
        if p.id.is_empty() {
            return Err("provider missing id".into());
        }
    }
    for m in &cfg.models {
        if m.id.is_empty() {
            return Err("model missing id".into());
        }
    }

    let mut hasher = Sha256::new();
    hasher.update(cwd.as_bytes());
    let hash = hex::encode(&hasher.finalize()[..8]);
    let store = Store::new(&format!("{}/{}", store_dir, hash))?;

    let (client, model_name) =
        resolve_client(&cfg.agent.model, &cfg.models, &cfg.providers, verbose)
            .map_err(|e| format!("main model: {}", e))?;

    let sp = if system_prompt.is_empty() {
        agent::build_system_prompt("", "", "", &cfg.agent.prompt, !cfg.agent.tools.is_empty())
    } else {
        system_prompt
    };

    let standard = tools::Registry::standard();
    let registry = if cfg.agent.tools.iter().any(|t| t == "*") {
        standard
    } else {
        let mut reg = tools::Registry::new();
        for name in &cfg.agent.tools {
            if let Some(t) = standard.get(name) {
                if let Err(e) = reg.register(t.clone()) {
                    log::warn!("registering tool tool={}: {}", name, e);
                }
            } else {
                log::warn!("unknown tool in config tool={}", name);
            }
        }
        reg
    };

    let subagents = resolve_subagents(&cfg, "", "", "", verbose);

    Ok(Deps {
        config: cfg,
        client,
        model_name,
        registry: Arc::new(registry),
        system_prompt: sp,
        cwd: cwd.to_string(),
        store,
        subagents,
        config_dir: String::new(),
        agent_name: String::new(),
        templates: Vec::new(),
    })
}

pub fn init(
    verbose: bool,
    config_dir: &str,
    tui: bool,
) -> Result<Deps, Box<dyn std::error::Error + Send + Sync>> {
    let cfg_dir = if config_dir.is_empty() {
        crate::config::dir()
    } else {
        config_dir.to_string()
    };

    let level = if verbose { "debug" } else { "info" };
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level));
    builder.format_timestamp_secs();
    if tui {
        let log_path = PathBuf::from(&cfg_dir).join("mate.log");
        let _ = std::fs::create_dir_all(&cfg_dir);
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
    }
    builder.try_init().ok();

    let cfg = crate::config::load_from(&cfg_dir)?;

    let system_md = read_file_if_exists(&format!("{}/SYSTEM.md", cfg_dir));
    let global_md = read_file_if_exists(&format!("{}/AGENTS.md", cfg_dir));
    let local_md = read_file_if_exists("AGENTS.md");

    let cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    let system_prompt = agent::build_system_prompt(
        &system_md,
        &global_md,
        &local_md,
        &cfg.agent.prompt,
        !cfg.agent.tools.is_empty(),
    );

    let session_dir = cfg.session.dir.clone();
    let mut deps = init_with_config(cfg, system_prompt, &cwd, &session_dir, verbose)?;

    deps.config_dir = cfg_dir.clone();

    deps.templates = load_templates(&cfg_dir);
    deps.subagents = resolve_subagents(&deps.config, &system_md, &global_md, &local_md, verbose);

    let skill_store = skills::load_skill_dirs(&deps.cwd, &cfg_dir)?;
    if let Some(reg) = Arc::get_mut(&mut deps.registry) {
        let _ = reg.register(skill_store.list_tool());
        let _ = reg.register(skill_store.load_tool());
    }

    Ok(deps)
}

fn load_templates(cfg_dir: &str) -> Vec<crate::prompts::Template> {
    let templates_dir = format!("{}/prompts", cfg_dir);
    match prompts::load(&templates_dir) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("loading prompt templates: {}", e);
            Vec::new()
        }
    }
}

fn read_file_if_exists(path: &str) -> String {
    match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("reading file path={}: {}", path, e);
            }
            String::new()
        }
    }
}
