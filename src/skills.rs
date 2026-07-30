use crate::tools::{Tool, define_tool};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: String,
    pub tools: Vec<String>,
}

pub struct Store {
    skills: Vec<Skill>,
}

impl Store {
    pub fn new() -> Self {
        Store { skills: Vec::new() }
    }

    pub fn load_dir(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let entries = match std::fs::read_dir(path) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Box::new(e)),
        };

        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let skill_path = entry.path();
            let skill_file = skill_path.join("SKILL.md");
            let data = match std::fs::read_to_string(&skill_file) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let skill = parse_skill_file(&data, &skill_path.to_string_lossy());
            self.skills.push(skill);
        }

        Ok(())
    }

    pub fn list(&self) -> String {
        if self.skills.is_empty() {
            return "No skills available.".to_string();
        }
        let mut out = String::new();
        for sk in &self.skills {
            out.push_str(&format!("- {}: {}", sk.name, sk.description));
            if !sk.tools.is_empty() {
                out.push_str(&format!(" (tools: {})", sk.tools.join(", ")));
            }
            out.push('\n');
        }
        out
    }

    pub fn load(&self, name: &str) -> Result<String, String> {
        for sk in &self.skills {
            if sk.name == name {
                return Ok(sk.content.clone());
            }
        }
        Err(format!("skill {:?} not found", name))
    }

    pub fn list_tool(&self) -> Tool {
        let skills_list = self.list();
        let params = crate::tools::object_schema(&[], &[]);
        define_tool(
            "list_skills",
            "List available skills with their descriptions and associated tools. Use this to discover domain-specific documentation before loading one with load_skill.",
            params,
            move |_: ListSkillsParams| {
                let skills_list = skills_list.clone();
                async move { Ok(skills_list) }
            },
        )
    }

    pub fn load_tool(&self) -> Tool {
        let skills = self.skills.clone();
        let params = crate::tools::object_schema(
            &[(
                "name",
                serde_json::json!({"type": "string", "description": "Name of the skill to load (as shown by list_skills)"}),
            )],
            &["name"],
        );
        define_tool(
            "load_skill",
            "Load the full content of a skill by name. Use list_skills first to see available skills and their descriptions.",
            params,
            move |p: LoadSkillParams| {
                let skills = skills.clone();
                async move {
                    for sk in &skills {
                        if sk.name == p.name {
                            return Ok(sk.content.clone());
                        }
                    }
                    Err(format!("skill {:?} not found", p.name))
                }
            },
        )
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

pub fn load_skill_dirs(
    cwd: &str,
    cfg_dir: &str,
) -> Result<Store, Box<dyn std::error::Error + Send + Sync>> {
    let mut store = Store::new();
    for dir in &[
        format!("{}/skills", cwd),
        format!("{}/.mate/skills", cwd),
        format!("{}/skills", cfg_dir),
    ] {
        if let Err(e) = store.load_dir(dir)
            && e.downcast_ref::<std::io::Error>()
                .is_none_or(|ioe| ioe.kind() != std::io::ErrorKind::NotFound)
        {
            log::warn!("loading skills dir {}: {}", dir, e);
        }
    }
    Ok(store)
}

fn parse_skill_file(raw: &str, source: &str) -> Skill {
    let mut skill = Skill {
        name: String::new(),
        description: String::new(),
        content: String::new(),
        source: source.to_string(),
        tools: Vec::new(),
    };

    let mut rest = raw;
    if let Some((frontmatter, body)) = crate::util::split_frontmatter(raw) {
        let fm: Frontmatter = yaml_serde::from_str(frontmatter).unwrap_or_default();
        skill.name = fm.name;
        skill.description = fm.description;
        skill.tools = fm.tools;
        rest = body;
    }

    if skill.name.is_empty() {
        skill.name = Path::new(source)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }
    if skill.description.is_empty() {
        skill.description = format!("Skill from {}", source);
    }

    skill.content = rest.trim().to_string();
    skill
}

#[derive(Debug, Deserialize)]
struct ListSkillsParams {}

#[derive(Debug, Deserialize)]
struct LoadSkillParams {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tools: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_new_empty() {
        let store = Store::new();
        assert_eq!(store.list(), "No skills available.");
    }

    #[test]
    fn test_load_dir_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new();
        store.load_dir(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(store.list(), "No skills available.");
    }

    #[test]
    fn test_load_dir_missing() {
        let mut store = Store::new();
        assert!(store.load_dir("/nonexistent/path/xyz").is_ok());
    }

    #[test]
    fn test_load_dir_with_skill() {
        let dir = tempfile::TempDir::new().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: Does things\n---\n\nSkill body here.",
        )
        .unwrap();

        let mut store = Store::new();
        store.load_dir(&dir.path().to_string_lossy()).unwrap();
        let list = store.list();
        assert!(list.contains("my-skill"));
        assert!(list.contains("Does things"));

        let content = store.load("my-skill").unwrap();
        assert_eq!(content, "Skill body here.");
    }

    #[test]
    fn test_load_missing_skill() {
        let store = Store::new();
        assert!(store.load("nonexistent").is_err());
    }

    #[test]
    fn test_skill_with_tools() {
        let dir = tempfile::TempDir::new().unwrap();
        let skill_dir = dir.path().join("rust-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: rust\ndescription: Rust help\ntools:\n  - bash\n  - read_file\n---\n\nRust content.",
        )
        .unwrap();

        let mut store = Store::new();
        store.load_dir(&dir.path().to_string_lossy()).unwrap();
        assert!(store.list().contains("(tools: bash, read_file)"));
    }

    #[test]
    fn test_list_tool() {
        let store = Store::new();
        let tool = store.list_tool();
        assert_eq!(tool.name, "list_skills");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((tool.execute)(serde_json::json!({}))).unwrap();
        assert_eq!(result, "No skills available.");
    }

    #[test]
    fn test_load_tool_not_found() {
        let store = Store::new();
        let tool = store.load_tool();
        assert_eq!(tool.name, "load_skill");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((tool.execute)(serde_json::json!({"name": "missing"})));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_tool_success() {
        let dir = tempfile::TempDir::new().unwrap();
        let skill_dir = dir.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: test-skill\ndescription: A test\n---\n\nTest content.",
        )
        .unwrap();

        let mut store = Store::new();
        store.load_dir(&dir.path().to_string_lossy()).unwrap();
        let tool = store.load_tool();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on((tool.execute)(serde_json::json!({"name": "test-skill"})))
            .unwrap();
        assert_eq!(result, "Test content.");
    }

    #[test]
    fn test_load_skill_dirs_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let store =
            load_skill_dirs(&dir.path().to_string_lossy(), &dir.path().to_string_lossy()).unwrap();
        assert_eq!(store.list(), "No skills available.");
    }
}
