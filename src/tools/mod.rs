mod bash;
mod edit_file;
pub mod gitignore;
mod read_file;
pub mod webfetch;
mod write_file;

use crate::message::ToolDef;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: BTreeMap<String, Value>,
    #[allow(clippy::type_complexity)]
    pub execute: Arc<
        dyn Fn(
                Value,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > + Send
            + Sync,
    >,
}

impl From<&Tool> for ToolDef {
    fn from(t: &Tool) -> Self {
        ToolDef {
            def_type: "function".to_string(),
            function: crate::message::ToolDefFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.parameters.clone(),
            },
        }
    }
}

#[derive(Clone)]
pub struct Registry {
    tools: HashMap<String, Tool>,
    order: Vec<String>,
}

impl Registry {
    pub fn new() -> Self {
        Registry {
            tools: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn standard() -> Self {
        let mut reg = Self::new();
        for t in standard() {
            let _ = reg.register(t);
        }
        reg
    }

    pub fn register(&mut self, tool: Tool) -> Result<(), String> {
        if self.tools.contains_key(&tool.name) {
            return Err(format!("tool {:?} already registered", tool.name));
        }
        self.order.push(tool.name.clone());
        self.tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.order.clone()
    }

    pub fn tool_defs(&self) -> Vec<ToolDef> {
        let mut defs = Vec::new();
        for name in &self.order {
            if let Some(t) = self.tools.get(name) {
                defs.push(ToolDef::from(t));
            }
        }
        defs
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn object_schema(
    props: &[(&str, serde_json::Value)],
    required: &[&str],
) -> BTreeMap<String, serde_json::Value> {
    let mut properties = serde_json::Map::new();
    for (k, v) in props {
        properties.insert((*k).to_string(), v.clone());
    }
    let mut map = BTreeMap::new();
    map.insert("type".to_string(), serde_json::json!("object"));
    map.insert(
        "properties".to_string(),
        serde_json::Value::Object(properties),
    );
    map.insert("required".to_string(), serde_json::json!(required));
    map
}

pub fn standard() -> Vec<Tool> {
    vec![
        bash::tool(),
        read_file::tool(),
        write_file::tool(),
        edit_file::tool(),
        webfetch::tool(),
    ]
}

const MAX_TOOL_OUTPUT_BYTES: usize = 200_000;

fn truncate_output(s: String) -> String {
    if s.len() <= MAX_TOOL_OUTPUT_BYTES {
        return s;
    }
    let mut cut = MAX_TOOL_OUTPUT_BYTES;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let dropped = s.len() - cut;
    let marker = format!(
        "\n... (output truncated: {} of {} bytes dropped, {} byte limit. Re-call with a smaller limit/offset to paginate.)",
        dropped,
        s.len(),
        MAX_TOOL_OUTPUT_BYTES
    );
    let mut out = String::with_capacity(cut + marker.len());
    out.push_str(&s[..cut]);
    out.push_str(&marker);
    out
}

pub fn define_tool<P, F, Fut>(
    name: &str,
    description: &str,
    params_schema: BTreeMap<String, Value>,
    f: F,
) -> Tool
where
    P: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(P) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let name = name.to_string();
    let description = description.to_string();
    Tool {
        name: name.clone(),
        description,
        parameters: params_schema,
        execute: Arc::new(
            move |raw: Value| -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<String, String>> + Send>,
            > {
                let result = serde_json::from_value::<P>(raw);
                let name = name.clone();
                let f = f.clone();
                Box::pin(async move {
                    let p = result
                        .map_err(|e| format!("invalid parameters for tool {}: {}", name, e))?;
                    let out = f(p).await?;
                    Ok(truncate_output(out))
                })
            },
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_new_registry() {
        let r = Registry::new();
        assert!(r.names().is_empty());
    }

    #[test]
    fn test_register() {
        let mut r = Registry::new();
        let tool = Tool {
            name: "test".to_string(),
            description: "desc".to_string(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        assert!(r.register(tool).is_ok());
        assert_eq!(r.names(), vec!["test"]);
    }

    #[test]
    fn test_register_duplicate() {
        let mut r = Registry::new();
        let tool = Tool {
            name: "dup".to_string(),
            description: String::new(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        let _ = r.register(tool);
        let tool2 = Tool {
            name: "dup".to_string(),
            description: String::new(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        assert!(r.register(tool2).is_err());
    }

    #[test]
    fn test_get() {
        let mut r = Registry::new();
        let tool = Tool {
            name: "findme".to_string(),
            description: "hello".to_string(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        let _ = r.register(tool);
        let found = r.get("findme");
        assert!(found.is_some());
        assert_eq!(found.unwrap().description, "hello");
        assert!(r.get("nope").is_none());
    }

    #[test]
    fn test_names_ordering() {
        let mut r = Registry::new();
        let make_tool = |n: &str| Tool {
            name: n.to_string(),
            description: String::new(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        let _ = r.register(make_tool("b"));
        let _ = r.register(make_tool("a"));
        let _ = r.register(make_tool("c"));
        assert_eq!(r.names(), vec!["b", "a", "c"]);
    }

    #[test]
    fn test_names_copy() {
        let mut r = Registry::new();
        let tool = Tool {
            name: "x".to_string(),
            description: String::new(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        let _ = r.register(tool);
        let mut names = r.names();
        names[0] = "hacked".to_string();
        assert_eq!(r.names()[0], "x");
    }

    #[test]
    fn test_tool_defs() {
        let mut r = Registry::new();
        let params = object_schema(&[("command", serde_json::json!({"type": "string"}))], &[]);

        let tool = Tool {
            name: "bash".to_string(),
            description: "run command".to_string(),
            parameters: params,
            execute: Arc::new(|_| Box::pin(async { Ok("ok".to_string()) })),
        };
        let _ = r.register(tool);
        let defs = r.tool_defs();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].def_type, "function");
        assert_eq!(defs[0].function.name, "bash");
    }

    #[test]
    fn test_tool_defs_empty() {
        let r = Registry::new();
        assert!(r.tool_defs().is_empty());
    }

    #[test]
    fn test_object_schema_key_order_stable() {
        let schema = object_schema(
            &[
                ("z_last", serde_json::json!({"type": "string"})),
                ("a_first", serde_json::json!({"type": "integer"})),
                ("m_mid", serde_json::json!({"type": "boolean"})),
            ],
            &["a_first", "z_last"],
        );
        let json = serde_json::to_string(&schema).unwrap();
        let expected = serde_json::to_string(&object_schema(
            &[
                ("z_last", serde_json::json!({"type": "string"})),
                ("a_first", serde_json::json!({"type": "integer"})),
                ("m_mid", serde_json::json!({"type": "boolean"})),
            ],
            &["a_first", "z_last"],
        ))
        .unwrap();
        assert_eq!(json, expected);
        assert_eq!(
            json,
            r#"{"properties":{"a_first":{"type":"integer"},"m_mid":{"type":"boolean"},"z_last":{"type":"string"}},"required":["a_first","z_last"],"type":"object"}"#
        );
    }

    #[test]
    fn test_standard_tool_defs_serialize_stable() {
        let a = Registry::standard().tool_defs();
        let b = Registry::standard().tool_defs();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn test_standard() {
        let tools = standard();
        assert_eq!(tools.len(), 5);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        for n in &["bash", "read_file", "write_file", "edit_file", "web_fetch"] {
            assert!(names.contains(n), "missing {}", n);
        }
    }

    #[derive(Debug, Deserialize)]
    struct GreetParams {
        name: String,
        #[allow(dead_code)]
        age: i32,
    }

    #[test]
    fn test_define_tool_valid_params() {
        let tool = define_tool(
            "greet",
            "says hello",
            BTreeMap::new(),
            |p: GreetParams| async move { Ok(format!("hello {}", p.name)) },
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((tool.execute)(
            serde_json::json!({"name": "Don", "age": 30}),
        ));
        assert_eq!(result.unwrap(), "hello Don");
    }

    #[test]
    fn test_define_tool_invalid_params() {
        let tool = define_tool(
            "greet",
            "says hello",
            BTreeMap::new(),
            |p: GreetParams| async move { Ok(format!("hello {}", p.name)) },
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((tool.execute)(serde_json::json!({})));
        assert!(result.is_err());
    }

    #[derive(Debug, Deserialize)]
    struct ContextParams {
        val: String,
    }

    #[test]
    fn test_define_tool_context_propagation() {
        let tool = define_tool(
            "ctx",
            "test",
            BTreeMap::new(),
            |p: ContextParams| async move { Ok(format!("got: {}", p.val)) },
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((tool.execute)(serde_json::json!({"val": "x"})));
        assert_eq!(result.unwrap(), "got: x");
    }

    #[test]
    fn test_registry_lookup() {
        let mut r = Registry::new();
        let tool = Tool {
            name: "reg_test".to_string(),
            description: "registry test".to_string(),
            parameters: BTreeMap::new(),
            execute: Arc::new(|_| Box::pin(async { Ok("found".to_string()) })),
        };
        let _ = r.register(tool);
        let found = r.get("reg_test");
        assert!(found.is_some());
        let t = found.unwrap();
        assert_eq!(t.name, "reg_test");
        assert_eq!(t.description, "registry test");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on((t.execute)(serde_json::json!({})));
        assert_eq!(result.unwrap(), "found");
    }

    #[test]
    fn test_registry_lookup_missing() {
        let r = Registry::new();
        assert!(r.get("nonexistent_tool_xyz").is_none());
    }

    #[test]
    fn test_standard_registry() {
        let r = Registry::standard();
        let names = r.names();
        for n in &["bash", "read_file", "write_file", "edit_file", "web_fetch"] {
            assert!(
                names.contains(&n.to_string()),
                "standard tool {:?} missing from registry: {:?}",
                n,
                names
            );
        }
    }

    #[test]
    fn test_truncate_output_under_limit() {
        let s = "a".repeat(100);
        assert_eq!(truncate_output(s.clone()), s);
    }

    #[test]
    fn test_truncate_output_at_limit() {
        let s = "a".repeat(MAX_TOOL_OUTPUT_BYTES);
        assert_eq!(truncate_output(s), "a".repeat(MAX_TOOL_OUTPUT_BYTES));
    }

    #[test]
    fn test_truncate_output_over_limit() {
        let s = "a".repeat(MAX_TOOL_OUTPUT_BYTES + 500);
        let out = truncate_output(s.clone());
        assert!(out.starts_with(&"a".repeat(MAX_TOOL_OUTPUT_BYTES)));
        assert!(out.contains("output truncated"));
        assert!(out.contains(&format!("{} byte limit", MAX_TOOL_OUTPUT_BYTES)));
        assert!(out.contains("500 of"));
        assert!(out.len() > s.len() - 500);
        assert!(out.len() < s.len());
    }

    #[test]
    fn test_truncate_output_multibyte_boundary() {
        let mut s = String::from("a").repeat(MAX_TOOL_OUTPUT_BYTES - 4);
        s.push_str("\u{1f600}\u{1f600}");
        let out = truncate_output(s);
        assert!(out.contains("output truncated"));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[tokio::test]
    async fn test_define_tool_applies_guard() {
        let tool = define_tool(
            "big",
            "returns big",
            BTreeMap::new(),
            |p: ContextParams| async move { Ok(p.val) },
        );
        let huge = "x".repeat(MAX_TOOL_OUTPUT_BYTES + 100);
        let result = (tool.execute)(serde_json::json!({"val": huge}))
            .await
            .unwrap();
        assert!(result.contains("output truncated"));
        assert!(result.contains("100 of"));
    }

    #[tokio::test]
    async fn test_define_tool_guard_passes_small() {
        let tool = define_tool(
            "small",
            "returns small",
            BTreeMap::new(),
            |p: ContextParams| async move { Ok(format!("ok {}", p.val)) },
        );
        let result = (tool.execute)(serde_json::json!({"val": "hi"}))
            .await
            .unwrap();
        assert_eq!(result, "ok hi");
    }
}
