use crate::agent::SubagentDef;
use crate::config::Config;
use crate::prompts::Template;
use crate::provider::Client;
use crate::session::store::Store;
use crate::skills::Store as SkillStore;
use crate::tools::Registry;
use std::collections::HashMap;
use std::sync::Arc;

pub mod bootstrap;
pub mod local;
pub mod resolve;
pub mod run;
pub use run::run_definition;
pub mod scheduler;
pub mod session;
pub mod session_manager;

pub struct Deps {
    pub agent_name: String,
    pub client: Client,
    pub registry: Arc<Registry>,
    pub system_prompt: String,
    pub cwd: String,
    pub store: Store,
    pub subagents: HashMap<String, SubagentDef>,
    pub skills: Option<SkillStore>,
    pub config: Config,
    pub config_dir: String,
    pub model_name: String,
    pub templates: Vec<Template>,
}

pub trait Interface: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, deps: Deps) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn notifier(&self, _deps: &Deps) -> Option<Arc<dyn Notifier + Send + Sync>> {
        None
    }
}

pub fn lookup_interface(name: &str) -> Option<Box<dyn Interface>> {
    match name {
        "local" => Some(Box::new(local::LocalInterface)),
        "slack" => Some(Box::new(crate::slack::BotAdapter)),
        "telegram" => Some(Box::new(crate::telegram::BotAdapter)),
        _ => None,
    }
}

pub trait Notifier: Send + Sync {
    fn schedule_notify(
        &self,
        channel: &str,
        message: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
