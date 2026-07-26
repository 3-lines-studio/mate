use serde_json;

pub const TOOL_COLOR: &str = crate::render::theme::VESPER.accent;

pub fn tool_pretty_name(name: &str) -> &str {
    match name {
        "read_file" => "read",
        "write_file" => "write",
        "edit_file" => "edit",
        _ => name,
    }
}

pub fn format_tool_label(cwd: &str, name: &str, args: &str) -> String {
    let raw = format_tool_label_raw(cwd, name, args);
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_tool_label_raw(cwd: &str, name: &str, args: &str) -> String {
    if args.is_empty() {
        return tool_pretty_name(name).to_string();
    }
    match name {
        "read_file" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let path = p["path"].as_str().unwrap_or("");
                let offset = p["offset"].as_i64().unwrap_or(0);
                let limit = p["limit"].as_i64().unwrap_or(0);
                if !path.is_empty() {
                    let label = rel_path(cwd, path);
                    if offset > 0 || limit > 0 {
                        let end = if limit > 0 {
                            format!("-{}", offset + limit - 1)
                        } else {
                            String::new()
                        };
                        return format!("read {} L{}{}", label, offset, end);
                    }
                    return format!("read {}", label);
                }
            }
        }
        "write_file" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let path = p["path"].as_str().unwrap_or("");
                let content = p["content"].as_str().unwrap_or("");
                if !path.is_empty() {
                    return format!("write {} {}B", rel_path(cwd, path), content.len());
                }
            }
        }
        "edit_file" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let path = p["path"].as_str().unwrap_or("");
                let edits = p["edits"].as_array().map(|a| a.len()).unwrap_or(0);
                if !path.is_empty() {
                    let unit = if edits == 1 { "edit" } else { "edits" };
                    return format!("edit {} {} {}", rel_path(cwd, path), edits, unit);
                }
            }
        }
        "bash" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let cmd = p["command"].as_str().unwrap_or("");
                if !cmd.is_empty() {
                    return cmd.to_string();
                }
            }
        }
        "delegate" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let subagent = p["subagent"].as_str().unwrap_or("");
                let task = p["task"].as_str().unwrap_or("");
                if !subagent.is_empty() {
                    let mut label = subagent.to_string();
                    if !task.is_empty() {
                        let t = if task.chars().count() > 80 {
                            let cut = task
                                .char_indices()
                                .take_while(|&(i, _)| i <= 80)
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            format!("{}…", &task[..cut])
                        } else {
                            task.to_string()
                        };
                        label = format!("{}: {}", label, t);
                    }
                    return label;
                }
            }
        }
        _ => {}
    }
    format!("{} {}", tool_pretty_name(name), args)
}

fn rel_path(cwd: &str, path: &str) -> String {
    if cwd.is_empty() {
        return path.to_string();
    }
    let cwd_path = std::path::Path::new(cwd);
    let p = std::path::Path::new(path);
    match p.strip_prefix(cwd_path) {
        Ok(rel) => {
            let s = rel.to_string_lossy();
            if s.is_empty() {
                ".".to_string()
            } else {
                s.to_string()
            }
        }
        Err(_) => path.to_string(),
    }
}

pub fn result_lang(tool_name: &str, args: &str) -> String {
    match tool_name {
        "bash" => "bash".to_string(),
        "read_file" => {
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(args) {
                let path = p["path"].as_str().unwrap_or("");
                if !path.is_empty() {
                    let ext = std::path::Path::new(path)
                        .extension()
                        .map(|e| e.to_string_lossy().to_string())
                        .unwrap_or_default();
                    return ext;
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tool_label_read_file() {
        let label = format_tool_label(
            "/home/user/project",
            "read_file",
            r#"{"path":"/home/user/project/src/main.rs","offset":10,"limit":5}"#,
        );
        assert_eq!(label, "read src/main.rs L10-14");
    }

    #[test]
    fn test_format_tool_label_bash() {
        let label = format_tool_label("/home/user", "bash", r#"{"command":"ls -la"}"#);
        assert_eq!(label, "ls -la");
    }

    #[test]
    fn test_format_tool_label_edit_file() {
        let label = format_tool_label(
            "/home/user/project",
            "edit_file",
            r#"{"path":"/home/user/project/src/main.rs","edits":[{},{}]}"#,
        );
        assert_eq!(label, "edit src/main.rs 2 edits");
    }

    #[test]
    fn test_format_tool_label_bash_multiline_command() {
        let label = format_tool_label("/home/user", "bash", r#"{"command":"echo foo\necho bar"}"#);
        assert_eq!(label, "echo foo echo bar");
        assert!(!label.contains('\n'));
    }

    #[test]
    fn test_format_tool_label_delegate_multiline_task() {
        let label = format_tool_label(
            "/home/user",
            "delegate",
            r#"{"subagent":"coder","task":"do this\nand that"}"#,
        );
        assert_eq!(label, "coder: do this and that");
        assert!(!label.contains('\n'));
    }

    #[test]
    fn test_format_tool_label_fallback_collapses_newlines() {
        let label = format_tool_label("/home/user", "custom", "{\"x\":\"a\nb\"}");
        assert!(!label.contains('\n'));
    }
}
