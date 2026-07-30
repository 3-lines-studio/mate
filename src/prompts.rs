use std::path::Path;

#[derive(Debug, Clone)]
pub struct Template {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub body: String,
}

pub fn load(dir: &str) -> Result<Vec<Template>, Box<dyn std::error::Error + Send + Sync>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Box::new(e)),
    };

    let mut templates = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") {
            continue;
        }
        if let Some(t) = parse_file(&entry.path())? {
            templates.push(t);
        }
    }
    Ok(templates)
}

fn parse_file(path: &Path) -> Result<Option<Template>, Box<dyn std::error::Error + Send + Sync>> {
    let data = std::fs::read_to_string(path)?;
    let name = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut t = Template {
        name,
        description: String::new(),
        argument_hint: String::new(),
        body: String::new(),
    };

    if let Some((fm, body)) = crate::util::split_frontmatter(&data) {
        let body = body.trim_start_matches('\r').trim_start_matches('\n');
        t.body = body.to_string();
        parse_frontmatter(fm, &mut t);
        return Ok(Some(t));
    }

    t.body = data.clone();
    t.description = first_line(&data);
    Ok(Some(t))
}

fn parse_frontmatter(fm: &str, t: &mut Template) {
    for line in fm.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once(':') {
            Some((k, v)) => (k.trim(), v.trim().trim_matches(&['"', '\''][..])),
            None => continue,
        };
        match key {
            "description" => t.description = value.to_string(),
            "argument-hint" => t.argument_hint = value.to_string(),
            _ => {}
        }
    }
}

fn first_line(s: &str) -> String {
    let s = s.trim();
    if let Some(idx) = s.find('\n') {
        s[..idx].to_string()
    } else {
        s.to_string()
    }
}

pub fn expand(t: &Template, args: &[String]) -> String {
    let mut body = t.body.clone();

    let mut has_placeholders = body.contains("$@") || body.contains("$ARGUMENTS");
    for i in 0..args.len() {
        if body.contains(&format!("${}", i + 1)) {
            has_placeholders = true;
            break;
        }
    }

    if !has_placeholders {
        if !args.is_empty() {
            body.push('\n');
            body.push_str(&args.join(" "));
        }
        return body;
    }

    for (i, arg) in args.iter().enumerate() {
        body = body.replace(&format!("${}", i + 1), arg);
    }
    let joined = args.join(" ");
    body = body.replace("$@", &joined);
    body = body.replace("$ARGUMENTS", &joined);

    body
}

pub fn find<'a>(templates: &'a [Template], name: &str) -> Option<&'a Template> {
    templates.iter().find(|t| t.name == name)
}

pub fn expand_text(templates: &[Template], text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && (i == 0 || is_text_space_char(chars[i - 1])) {
            let mut j = i + 1;
            while j < chars.len() && !is_text_space_char(chars[j]) {
                j += 1;
            }
            let cmd: String = chars[i + 1..j].iter().collect();
            if let Some(t) = find(templates, &cmd) {
                let mut k = j;
                while k < chars.len() && chars[k] != '\n' {
                    k += 1;
                }
                let rest: String = chars[j..k].iter().collect();
                let rest = rest.trim_start_matches(&[' ', '\t'][..]);
                let args: Vec<String> = if rest.is_empty() {
                    Vec::new()
                } else {
                    rest.split_whitespace().map(|s| s.to_string()).collect()
                };
                let expanded = expand(t, &args);
                result.push_str(&expanded);
                i = k.saturating_sub(1);
            } else {
                result.push(chars[i]);
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}

fn is_text_space_char(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_empty_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let templates = load(&dir.path().to_string_lossy()).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn test_load_missing_dir() {
        let templates = load("/nonexistent/path/xyz").unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn test_load_md_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("greet.md"), "Hello, world!").unwrap();
        let templates = load(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "greet");
        assert_eq!(templates[0].body, "Hello, world!");
    }

    #[test]
    fn test_load_with_frontmatter() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("review.md"),
            "---\ndescription: Review code\nargument-hint: path\n---\nReview this file",
        )
        .unwrap();
        let templates = load(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "review");
        assert_eq!(templates[0].description, "Review code");
        assert_eq!(templates[0].argument_hint, "path");
        assert_eq!(templates[0].body, "Review this file");
    }

    #[test]
    fn test_expand_no_placeholders() {
        let t = Template {
            name: "hello".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Hello".to_string(),
        };
        let result = expand(&t, &["world".to_string()]);
        assert_eq!(result, "Hello\nworld");
    }

    #[test]
    fn test_expand_positional() {
        let t = Template {
            name: "greet".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Hello $1, you are $2 years old".to_string(),
        };
        let result = expand(&t, &["Alice".to_string(), "30".to_string()]);
        assert_eq!(result, "Hello Alice, you are 30 years old");
    }

    #[test]
    fn test_expand_all_args() {
        let t = Template {
            name: "echo".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Args: $@".to_string(),
        };
        let result = expand(&t, &["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(result, "Args: a b c");
    }

    #[test]
    fn test_expand_text_expansion() {
        let t = Template {
            name: "hello".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Hello $1!".to_string(),
        };
        let templates = vec![t];
        let result = expand_text(&templates, "say /hello world to everyone");
        assert_eq!(result, "say Hello world!");
    }

    #[test]
    fn test_expand_text_no_match() {
        let templates: Vec<Template> = vec![];
        let result = expand_text(&templates, "/hello world");
        assert_eq!(result, "/hello world");
    }

    #[test]
    fn test_expand_text_not_after_space() {
        let t = Template {
            name: "x".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Y".to_string(),
        };
        let templates = vec![t];
        let result = expand_text(&templates, "a/x b");
        assert_eq!(result, "a/x b");
    }

    #[test]
    fn test_expand_text_without_args_appends() {
        let t = Template {
            name: "todo".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Write a function that".to_string(),
        };
        let templates = vec![t];
        let result = expand_text(&templates, "/todo calculates the sum of two ints");
        assert!(result.starts_with("Write a function that\ncalculates the sum of two ints"));
    }

    #[test]
    fn test_expand_text_multibyte_no_corruption() {
        // Multi-byte UTF-8 before the slash must not corrupt char boundaries.
        let t = Template {
            name: "hi".to_string(),
            description: String::new(),
            argument_hint: String::new(),
            body: "Hello $1!".to_string(),
        };
        let templates = vec![t];

        // café (é = 2 bytes) right before the space that precedes /hi
        let result = expand_text(&templates, "café /hi 世界");
        assert_eq!(result, "café Hello 世界!");
    }

    #[test]
    fn test_expand_text_multibyte_in_command_name_not_matched() {
        // A multi-byte char that looks like '/' in byte form is still fine.
        let templates: Vec<Template> = vec![];
        let result = expand_text(&templates, "héllo /cmd");
        // No template named "cmd", so the slash is preserved as-is.
        assert_eq!(result, "héllo /cmd");
    }
}
