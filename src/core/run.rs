use super::{Deps, bootstrap, resolve, scheduler};
use crate::tools::Tool;
use std::sync::Arc;

pub fn run_definition(
    agent_name: &str,
    config_dir: &str,
    extra_tools: fn(&crate::config::Config, &str) -> Vec<Tool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut verbose = false;
    let mut local = false;
    let mut debug_prompts = false;
    let mut model = String::new();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => verbose = true,
            "--local" => local = true,
            "--debug-prompts" => debug_prompts = true,
            "-m" | "--model" if i + 1 < args.len() => {
                model = args[i + 1].clone();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let mut deps = bootstrap::init(verbose, config_dir, false)?;

    if debug_prompts {
        print_debug_prompts(&mut deps);
        return Ok(());
    }

    if !model.is_empty() {
        match resolve::resolve_client(&model, &deps.config.models, &deps.config.providers, verbose)
        {
            Ok((client, model_name)) => {
                deps.client = client;
                deps.model_name = model_name;
            }
            Err(e) => return Err(format!("model: {}", e).into()),
        }
    }

    deps.agent_name = agent_name.to_string();

    let tools = extra_tools(&deps.config, &deps.config_dir);
    if let Some(reg) = Arc::get_mut(&mut deps.registry) {
        for t in tools {
            if let Err(e) = reg.register(t) {
                log::warn!("registering tool: {}", e);
            }
        }
    }

    if !local {
        let interface_names = if deps.config.agent.interfaces.is_empty() {
            vec!["local".to_string()]
        } else {
            deps.config.agent.interfaces.clone()
        };

        for name in &interface_names {
            if name == "local" {
                continue;
            }
            let iface = match super::lookup_interface(name) {
                Some(i) => i,
                None => return Err(format!("interface \"{}\" not registered", name).into()),
            };
            if !deps.config.schedule.jobs.is_empty() {
                match iface.notifier(&deps) {
                    Some(notifier) => scheduler::start_scheduler(&deps, notifier),
                    None => {
                        return Err(format!(
                            "schedule configured but interface \"{}\" has no notifier",
                            name
                        )
                        .into());
                    }
                }
            }
            log::info!("{} starting", agent_name);
            iface.run(deps)?;
            return Ok(());
        }
    }

    let mut app = crate::tui::App::new(deps);
    app.run()?;
    Ok(())
}

fn print_debug_prompts(deps: &mut Deps) {
    let sess = deps.store.create().unwrap();
    let tmp = deps.new_session(sess);

    eprintln!("=== Main Agent System Prompt ===");
    eprintln!("{}", tmp.system_prompt());

    eprintln!("\n=== Main Agent Tool Definitions ===");
    for td in tmp.tool_defs() {
        eprintln!("  {}", serde_json::to_string_pretty(&td).unwrap());
    }

    if !deps.subagents.is_empty() {
        eprintln!("\n=== Subagents ===");
        let mut ids: Vec<&String> = deps.subagents.keys().collect();
        ids.sort();
        for id in ids {
            let def = &deps.subagents[id];
            eprintln!("\n--- Subagent: {} ---", id);
            eprintln!("Model: {}", def.model_name);
            eprintln!("Description: {}", def.description);
            eprintln!("System Prompt:\n{}", def.system_prompt);
            eprintln!("Tools: {:?}", def.registry.names());
        }
    }
}
