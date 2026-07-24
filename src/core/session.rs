use crate::agent::{AgentSession, SubagentDef};
use crate::provider::Client;
use crate::session::store::Store;
use crate::session::{Cache, KeyStore, Session};
use crate::tools::Registry;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;

use super::Deps;
use super::session_manager::SessionManager;

struct SessionParts {
    store: Store,
    registry: Arc<Registry>,
    system_prompt: String,
    cwd: String,
    subagents: HashMap<String, SubagentDef>,
    compaction_client: Option<Client>,
}

impl Deps {
    fn session_parts(&self) -> SessionParts {
        SessionParts {
            store: self.store.clone(),
            registry: self.registry.clone(),
            system_prompt: self.system_prompt.clone(),
            cwd: self.cwd.clone(),
            subagents: self.subagents.clone(),
            compaction_client: self.compaction_client.clone(),
        }
    }

    pub fn new_session(&self, sess: Session) -> AgentSession {
        self.new_session_with_client(sess, &self.client)
    }

    pub fn new_session_with_client(&self, sess: Session, client: &Client) -> AgentSession {
        let parts = self.session_parts();
        Self::build_session(&parts, sess, client)
    }

    fn build_session(parts: &SessionParts, sess: Session, client: &Client) -> AgentSession {
        let mut asession = AgentSession::new(
            Arc::new(TokioMutex::new(parts.store.clone())),
            sess,
            Arc::new(client.clone()),
            parts.registry.clone(),
            parts.system_prompt.clone(),
            parts.cwd.clone(),
        );
        asession.set_subagents(parts.subagents.clone());
        if let Some(cc) = &parts.compaction_client {
            asession.set_compaction_client(Arc::new(cc.clone()));
        }
        asession
    }

    pub fn new_session_manager(
        &self,
        key_store_path: &str,
    ) -> Result<SessionManager, Box<dyn std::error::Error + Send + Sync>> {
        let ks = KeyStore::new(key_store_path)?;
        let cache = Cache::new(50);
        let parts = self.session_parts();
        let client = self.client.clone();
        let factory = Box::new(move |sess: Session| Deps::build_session(&parts, sess, &client));
        Ok(SessionManager::new(self.store.clone(), cache, ks, factory))
    }
}
