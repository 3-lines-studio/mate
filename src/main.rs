use mate::core::{self, bootstrap};
use mate::prompts;
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (subcmd, _before, after) = find_subcommand(&args[1..]);

    match subcmd {
        "run" => run_cmd(&after),
        "clean" => clean_cmd(&after),
        _ => default_cmd(&args[1..]),
    }
}

fn find_subcommand(args: &[String]) -> (&str, Vec<String>, Vec<String>) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-m" || a == "--model" {
            if i + 1 < args.len() {
                i += 1;
            }
            i += 1;
            continue;
        }
        if a.starts_with("-m=") || a.starts_with("--model=") {
            i += 1;
            continue;
        }
        if !a.starts_with('-') && (a == "run" || a == "clean") {
            let before: Vec<String> = args[..i].to_vec();
            let after: Vec<String> = args[i + 1..].to_vec();
            return (a.as_str(), before, after);
        }
        i += 1;
    }
    ("", vec![], args.to_vec())
}

fn extract_flags(args: &[String]) -> (bool, bool, bool, String, Vec<String>) {
    let mut verbose = false;
    let mut local = false;
    let mut debug_prompts = false;
    let mut model = String::new();
    let mut cleaned = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => verbose = true,
            "--local" => local = true,
            "--debug-prompts" => debug_prompts = true,
            "-m" | "--model" => {
                if i + 1 < args.len() {
                    model = args[i + 1].clone();
                    i += 1;
                }
            }
            _ => cleaned.push(args[i].clone()),
        }
        i += 1;
    }
    (verbose, local, debug_prompts, model, cleaned)
}

fn run_cmd(args: &[String]) {
    let (verbose, _local, debug_prompts, model, cleaned) = extract_flags(args);
    let prompt = cleaned.join(" ");

    let mut deps = match bootstrap::init(verbose, "", false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("mate run: {}", e);
            std::process::exit(1);
        }
    };

    if debug_prompts {
        print_debug_prompts(&mut deps);
        return;
    }

    if !model.is_empty() {
        match mate::core::resolve::resolve_client(
            &model,
            &deps.config.models,
            &deps.config.providers,
            verbose,
        ) {
            Ok((client, model_name)) => {
                deps.client = client;
                deps.model_name = model_name;
            }
            Err(e) => {
                eprintln!("mate run: model: {}", e);
                std::process::exit(1);
            }
        }
    }

    let is_pipe = !is_stdin_tty();
    let mut stdin_content = String::new();
    if is_pipe {
        std::io::stdin().read_to_string(&mut stdin_content).unwrap();
        stdin_content = stdin_content.trim().to_string();
    }

    if prompt.is_empty() && stdin_content.is_empty() {
        eprintln!("usage: mate run [prompt or /template]");
        eprintln!("  reads from stdin when piped");
        std::process::exit(1);
    }

    let mut final_prompt = prompt.clone();
    if !final_prompt.is_empty() {
        final_prompt = prompts::expand_text(&deps.templates, &final_prompt);
    }

    if !stdin_content.is_empty() {
        if !final_prompt.is_empty() {
            final_prompt.push_str("\n\n");
            final_prompt.push_str(&stdin_content);
        } else {
            final_prompt = stdin_content;
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    let _enter = rt.enter();

    let sess = deps.store.create().unwrap();
    let mut asession = deps.new_session(sess);

    let render_md = is_stdout_tty();
    let term_w = terminal_width();
    let mut md_buf = String::new();

    let mut events = asession.prompt(&final_prompt);
    let mut failed = false;
    while let Some(ev) = rt.block_on(events.recv()) {
        match &ev.kind {
            mate::agent::EventKind::TextDelta(delta) => {
                if render_md {
                    md_buf.push_str(delta);
                } else {
                    print!("{}", delta);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            }
            mate::agent::EventKind::ToolError { name, error, .. } => {
                eprintln!("❌ {}: {}", name, error);
            }
            mate::agent::EventKind::Error(msg) => {
                eprintln!("Error: {}", msg);
                failed = true;
                break;
            }
            mate::agent::EventKind::AgentDone(_) => {
                if render_md && !md_buf.is_empty() {
                    let renderer = mate::render::StreamRenderer::new(term_w);
                    println!("{}", renderer.render(&md_buf));
                    md_buf.clear();
                } else {
                    println!();
                }
            }
            _ => {}
        }
    }
    mate::herdr::shutdown();
    if failed {
        std::process::exit(1);
    }
}

fn clean_cmd(args: &[String]) {
    let (verbose, _local, _, _, cleaned) = extract_flags(args);

    let all = cleaned.contains(&"--all".to_string());
    let dry_run = cleaned.contains(&"--dry-run".to_string());
    let older_than: Option<usize> = cleaned
        .iter()
        .position(|a| a == "--older-than")
        .and_then(|i| cleaned.get(i + 1))
        .and_then(|s| s.parse().ok());

    let mut deps = match bootstrap::init(verbose, "", false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("mate clean: {}", e);
            std::process::exit(1);
        }
    };

    let sessions = match deps.store.list() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mate clean: {}", e);
            std::process::exit(1);
        }
    };

    if sessions.is_empty() {
        println!("No sessions.");
        return;
    }

    let mut sessions = sessions;
    sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));

    let mut to_delete = Vec::new();
    let mut total_bytes: i64 = 0;

    for s in &sessions {
        let age_days = (chrono::Utc::now().timestamp() - s.updated_at.timestamp()) / 86400;
        let size = session_size(&deps.store.dir(), &s.id);

        let delete_it = all || older_than.is_some_and(|ot| ot > 0 && age_days as usize >= ot);

        if !delete_it {
            println!(
                "{}  {:>6}  {:>4} turns  {:>3}d  {}",
                s.id,
                format_bytes(size),
                s.turn_count,
                age_days,
                s.name
            );
            continue;
        }

        to_delete.push(s.clone());
        total_bytes += size;
        println!(
            "DELETE {}  {:>6}  {:>4} turns  {:>3}d  {}",
            s.id,
            format_bytes(size),
            s.turn_count,
            age_days,
            s.name
        );
    }

    if to_delete.is_empty() {
        return;
    }

    println!("\nTotal to delete: {}", format_bytes(total_bytes));

    if dry_run {
        return;
    }

    print!("Confirm? [y/N] ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let mut resp = String::new();
    std::io::stdin().read_line(&mut resp).unwrap();
    if resp.trim().to_lowercase() != "y" {
        println!("Aborted.");
        return;
    }

    for s in &to_delete {
        if let Err(e) = deps.store.delete(&s.id) {
            eprintln!("delete {}: {}", s.id, e);
        }
    }
}

fn default_cmd(args: &[String]) {
    let (verbose, local, debug_prompts, model, _) = extract_flags(args);

    let mut deps = match bootstrap::init(verbose, "", true) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("mate: {}", e);
            std::process::exit(1);
        }
    };

    if debug_prompts {
        print_debug_prompts(&mut deps);
        return;
    }

    if !model.is_empty() {
        match mate::core::resolve::resolve_client(
            &model,
            &deps.config.models,
            &deps.config.providers,
            verbose,
        ) {
            Ok((client, model_name)) => {
                deps.client = client;
                deps.model_name = model_name;
            }
            Err(e) => {
                eprintln!("mate: model: {}", e);
                std::process::exit(1);
            }
        }
    }

    deps.agent_name = "mate".to_string();

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
            let iface = match core::lookup_interface(name) {
                Some(i) => i,
                None => {
                    eprintln!("mate: interface \"{}\" not registered", name);
                    std::process::exit(1);
                }
            };
            if let Err(e) = iface.run(deps) {
                eprintln!("mate: {}", e);
                std::process::exit(1);
            }
            return;
        }
    }

    let mut app = mate::tui::App::new(deps);
    if let Err(e) = app.run() {
        eprintln!("mate: {}", e);
        std::process::exit(1);
    }
}

fn print_debug_prompts(deps: &mut core::Deps) {
    eprintln!("=== Main Agent System Prompt ===");
    let sess = deps.store.create().unwrap();
    let tmp = deps.new_session(sess);
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

fn session_size(dir: &str, id: &str) -> i64 {
    let path = std::path::PathBuf::from(dir).join(id);
    walkdir_size(&path)
}

fn walkdir_size(path: &std::path::Path) -> i64 {
    let mut total: i64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += walkdir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len() as i64;
            }
        }
    }
    total
}

fn format_bytes(b: i64) -> String {
    const UNIT: i64 = 1024;
    if b < UNIT {
        return format!("{} B", b);
    }
    let mut div = UNIT;
    let mut exp = 0;
    let mut n = b / UNIT;
    while n >= UNIT {
        div *= UNIT;
        exp += 1;
        n /= UNIT;
    }
    format!(
        "{:.1} {}B",
        b as f64 / div as f64,
        "KMGTPE".as_bytes()[exp] as char
    )
}

#[cfg(unix)]
fn is_stdin_tty() -> bool {
    unsafe { libc::isatty(0) != 0 }
}

#[cfg(not(unix))]
fn is_stdin_tty() -> bool {
    false
}

#[cfg(unix)]
fn is_stdout_tty() -> bool {
    unsafe { libc::isatty(1) != 0 }
}

#[cfg(not(unix))]
fn is_stdout_tty() -> bool {
    false
}

#[cfg(unix)]
fn terminal_width() -> usize {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let rc = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) };
    if rc == 0 && ws.ws_col > 0 {
        ws.ws_col as usize
    } else {
        80
    }
}

#[cfg(not(unix))]
fn terminal_width() -> usize {
    80
}
