use globset::Glob;
use std::path::Path;

#[derive(Debug, Clone)]
struct GitignoreRule {
    pattern: String,
    negate: bool,
    dir_only: bool,
    anchored: bool,
    dir: String,
    glob: Option<Glob>,
}

#[derive(Debug, Clone)]
pub struct GitignoreMatcher {
    rules: Vec<GitignoreRule>,
}

pub(crate) fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "vendor" | ".idea" | ".vscode" | "__pycache__" | ".pytest_cache"
    )
}

fn parse_gitignore_file(path: &str, dir: &str) -> Vec<GitignoreRule> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut rules = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut negate = false;
        let mut line = line.to_string();
        if line.starts_with('!') {
            negate = true;
            line = line[1..].to_string();
        }

        let mut dir_only = false;
        if line.ends_with('/') {
            dir_only = true;
            line.pop();
        }

        let mut anchored = false;
        if let Some(stripped) = line.strip_prefix('/') {
            anchored = true;
            line = stripped.to_string();
        }

        let matcher = globset::GlobBuilder::new(&line)
            .literal_separator(true)
            .build()
            .ok();

        rules.push(GitignoreRule {
            pattern: line,
            negate,
            dir_only,
            anchored,
            dir: dir.to_string(),
            glob: matcher,
        });
    }
    rules
}

impl GitignoreMatcher {
    pub(crate) fn new() -> Self {
        GitignoreMatcher { rules: Vec::new() }
    }

    pub(crate) fn add_file(&mut self, path: &str, dir: &str) {
        self.rules.extend(parse_gitignore_file(path, dir));
    }

    pub fn is_ignored(&self, rel_path: &str, is_dir: bool) -> bool {
        let mut ignored = false;
        for rule in &self.rules {
            if !is_dir && rule.dir_only {
                continue;
            }

            if !rule.dir.is_empty()
                && !rel_path.starts_with(&format!("{}/", rule.dir))
                && rel_path != rule.dir
            {
                continue;
            }

            let target = if rule.anchored {
                if rule.dir.is_empty() {
                    rel_path.to_string()
                } else {
                    rel_path
                        .strip_prefix(&format!("{}/", rule.dir))
                        .unwrap_or(rel_path)
                        .to_string()
                }
            } else if !rule.pattern.contains('/') {
                Path::new(rel_path)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel_path.to_string())
            } else if !rule.dir.is_empty() {
                rel_path
                    .strip_prefix(&format!("{}/", rule.dir))
                    .unwrap_or(rel_path)
                    .to_string()
            } else {
                rel_path.to_string()
            };

            if let Some(g) = &rule.glob
                && g.compile_matcher().is_match(&target)
            {
                ignored = !rule.negate;
            }
        }
        ignored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher_from(dir: &Path, relative_gi: &str, content: &str) -> GitignoreMatcher {
        let gi_path = if relative_gi.is_empty() {
            dir.join(".gitignore")
        } else {
            dir.join(relative_gi).join(".gitignore")
        };
        if let Some(parent) = gi_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&gi_path, content).unwrap();
        let mut m = GitignoreMatcher::new();
        let dir_rel = if relative_gi.is_empty() {
            String::new()
        } else {
            relative_gi.to_string()
        };
        m.add_file(&gi_path.to_string_lossy(), &dir_rel);
        m
    }

    #[test]
    fn test_gitignore_basic() {
        let dir = tempfile::TempDir::new().unwrap();
        let matcher = matcher_from(dir.path(), "", "ignored_dir/\n*.log\n");

        assert!(matcher.is_ignored("ignored_dir", true));
        assert!(matcher.is_ignored("debug.log", false));
        assert!(!matcher.is_ignored("main.go", false));
    }

    #[test]
    fn test_gitignore_negation() {
        let dir = tempfile::TempDir::new().unwrap();
        let matcher = matcher_from(dir.path(), "", "*.go\n!important.go\n");

        assert!(matcher.is_ignored("main.go", false));
        assert!(!matcher.is_ignored("important.go", false));
    }

    #[test]
    fn test_gitignore_nested() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut matcher = matcher_from(dir.path(), "", "*.log\n");
        let sub_gi = dir.path().join("sub").join(".gitignore");
        std::fs::create_dir_all(sub_gi.parent().unwrap()).unwrap();
        std::fs::write(&sub_gi, "*.go\n").unwrap();
        matcher.add_file(&sub_gi.to_string_lossy(), "sub");

        assert!(matcher.is_ignored("debug.log", false));
        assert!(matcher.is_ignored("sub/helper.go", false));
        assert!(!matcher.is_ignored("main.go", false));
        assert!(matcher.is_ignored("sub/data.log", false));
    }

    #[test]
    fn test_gitignore_leading_slash_anchored() {
        let dir = tempfile::TempDir::new().unwrap();
        let matcher = matcher_from(dir.path(), "", "/target\n");

        assert!(matcher.is_ignored("target", true));
        assert!(!matcher.is_ignored("sub/target", true));
    }

    #[test]
    fn test_gitignore_no_file() {
        let matcher = GitignoreMatcher::new();
        assert!(!matcher.is_ignored("anything.go", false));
    }
}
