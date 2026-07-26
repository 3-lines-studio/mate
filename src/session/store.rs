#[cfg(test)]
use crate::message::Message;
use crate::session::cache::Cache;
#[cfg(test)]
use crate::session::types::compute_turn_id;
use crate::session::types::{Session, Turn, TurnMeta, turn_label};
use chrono::Utc;
use rand::RngExt;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
    index_cache: Cache<Vec<TurnMeta>>,
}

impl Store {
    pub fn new(dir: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dir = expand_tilde(dir);
        let dir_path = PathBuf::from(&dir);
        std::fs::create_dir_all(&dir_path)?;
        Ok(Self {
            dir: dir_path,
            index_cache: Cache::new(64),
        })
    }

    pub fn dir(&self) -> String {
        self.dir.to_string_lossy().to_string()
    }

    pub fn create(&mut self) -> Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let id = Utc::now().timestamp_millis().to_string();
        let hash = random_hash();
        let now = Utc::now();
        let sess = Session {
            id: id.clone(),
            name: hash.clone(),
            hash,
            named: false,
            current_turn: String::new(),
            created_at: now,
            updated_at: now,
            turn_count: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            cost: 0.0,
        };
        self.save_meta_locked(&sess)?;
        Ok(sess)
    }

    pub fn load(&mut self, id: &str) -> Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        self.read_meta(id)
    }

    pub fn save_meta(
        &mut self,
        sess: &Session,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sess = sess.clone();
        sess.updated_at = Utc::now();
        self.save_meta_locked(&sess)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dir = self.session_dir(id);
        if !dir.exists() {
            return Ok(());
        }
        self.index_cache.remove(id);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Session>, Box<dyn std::error::Error + Send + Sync>> {
        let mut sessions = Vec::new();
        let entries = std::fs::read_dir(&self.dir)?;
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(meta) = self.read_meta(&name) {
                sessions.push(meta);
            }
        }
        Ok(sessions)
    }

    pub fn set_name(&self, sess: &mut Session, first_message: &str) {
        if sess.named {
            return;
        }
        let clean = first_message.replace('\n', " ");
        sess.name = format!(
            "{} - {}",
            sess.hash,
            crate::util::truncate_with_ellipsis(&clean, 40, "...")
        );
        sess.named = true;
    }

    pub fn commit_turn(
        &mut self,
        session_id: &str,
        turn: &Turn,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dir = self.turns_dir(session_id);
        std::fs::create_dir_all(&dir)?;

        let turn_path = self.turn_path(session_id, &turn.id);
        if turn_path.exists() {
            return Err(
                format!("turn {} already exists in session {}", turn.id, session_id).into(),
            );
        }

        let data = serde_json::to_string(turn)?;
        atomic_write(&turn_path, &data)?;

        let meta = TurnMeta {
            id: turn.id.clone(),
            parent_id: turn.parent_id.clone(),
            label: turn_label(&turn.messages),
            created_at: turn.created_at,
            subagent: turn.subagent.clone(),
            tool_call_id: turn.tool_call_id.clone(),
        };

        self.append_index(session_id, &meta)?;
        Ok(())
    }

    pub fn load_turn(
        &mut self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Turn, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.turn_path(session_id, turn_id);
        let data = std::fs::read_to_string(&path)?;
        let turn: Turn = serde_json::from_str(&data)?;
        Ok(turn)
    }

    pub fn turn_index(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<TurnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        self.load_index(session_id)
    }

    pub fn ancestry(
        &mut self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Vec<Turn>, Box<dyn std::error::Error + Send + Sync>> {
        let index = self.load_index(session_id)?;

        let by_id: HashMap<String, &TurnMeta> = index.iter().map(|m| (m.id.clone(), m)).collect();

        let mut chain: Vec<String> = Vec::new();
        let mut current = turn_id.to_string();
        while !current.is_empty() {
            chain.push(current.clone());
            match by_id.get(&current) {
                Some(meta) => current = meta.parent_id.clone(),
                None => break,
            }
        }

        let mut turns = Vec::with_capacity(chain.len());
        for id in chain.iter().rev() {
            let turn = self.load_turn_locked(session_id, id)?;
            turns.push(turn);
        }
        Ok(turns)
    }

    fn save_meta_locked(
        &self,
        sess: &Session,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let dir = self.session_dir(&sess.id);
        std::fs::create_dir_all(&dir)?;
        let path = self.meta_path(&sess.id);
        let data = serde_json::to_string(sess)?;
        atomic_write(&path, &data)?;
        Ok(())
    }

    fn read_meta(&self, id: &str) -> Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.meta_path(id);
        let data = std::fs::read_to_string(&path)?;
        let sess: Session = serde_json::from_str(&data)?;
        Ok(sess)
    }

    fn load_index(
        &mut self,
        session_id: &str,
    ) -> Result<Vec<TurnMeta>, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(cached) = self.index_cache.get(session_id) {
            return Ok(cached.clone());
        }

        let path = self.index_path(session_id);
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(Box::new(e)),
        };

        let mut entries = Vec::new();
        for line in data.lines() {
            if line.is_empty() {
                continue;
            }
            let meta: TurnMeta = serde_json::from_str(line)?;
            entries.push(meta);
        }

        self.index_cache.put(session_id, entries.clone());
        Ok(entries)
    }

    fn append_index(
        &mut self,
        session_id: &str,
        meta: &TurnMeta,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let path = self.index_path(session_id);
        let mut line = serde_json::to_string(meta)?;
        line.push('\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?
            .write_all(line.as_bytes())?;

        if let Some(mut cached) = self.index_cache.get(session_id).cloned() {
            cached.push(meta.clone());
            self.index_cache.remove(session_id);
            self.index_cache.put(session_id, cached);
        }
        Ok(())
    }

    fn load_turn_locked(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Turn, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.turn_path(session_id, turn_id);
        let data = std::fs::read_to_string(&path)?;
        let turn: Turn = serde_json::from_str(&data)?;
        Ok(turn)
    }

    fn session_dir(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    fn meta_path(&self, id: &str) -> PathBuf {
        self.session_dir(id).join("meta.json")
    }

    fn turns_dir(&self, id: &str) -> PathBuf {
        self.session_dir(id).join("turns")
    }

    fn index_path(&self, id: &str) -> PathBuf {
        self.turns_dir(id).join("index.jsonl")
    }

    fn turn_path(&self, session_id: &str, turn_id: &str) -> PathBuf {
        self.turns_dir(session_id).join(format!("{}.json", turn_id))
    }
}

fn random_hash() -> String {
    let bytes: [u8; 3] = rand::rng().random();
    hex::encode(bytes)
}

fn atomic_write(
    path: &std::path::Path,
    data: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, data)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn expand_tilde(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix('~')
        && let Ok(home) = std::env::var("HOME")
    {
        let mut p = PathBuf::from(home);
        if !stripped.is_empty() {
            p.push(&stripped[1..]);
        }
        return p.to_string_lossy().to_string();
    }
    path.to_string()
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Role;

    fn make_msg(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            reasoning_content: String::new(),
            reasoning_details: vec![],
            tool_calls: vec![],
            tool_call_id: String::new(),
            name: String::new(),
            tool_duration: String::new(),
        }
    }

    fn commit_turn(
        store: &mut Store,
        session_id: &str,
        parent_id: &str,
        user_content: &str,
        assistant_content: &str,
    ) -> Turn {
        let msgs = vec![
            make_msg(Role::User, user_content),
            make_msg(Role::Assistant, assistant_content),
        ];
        let id = compute_turn_id(parent_id, &msgs);
        let turn = Turn {
            id,
            parent_id: parent_id.to_string(),
            messages: msgs,
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };
        store.commit_turn(session_id, &turn).unwrap();
        turn
    }

    #[test]
    fn test_new_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::new(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(store.dir(), dir.path().to_string_lossy().to_string());
    }

    #[test]
    fn test_new_store_creates_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let new_dir = dir.path().join("new-sub");
        Store::new(&new_dir.to_string_lossy()).unwrap();
        assert!(new_dir.exists());
    }

    #[test]
    fn test_create() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();
        assert!(!sess.id.is_empty());
        assert!(!sess.name.is_empty());
        assert!(!sess.hash.is_empty());
        assert_eq!(sess.hash.len(), 6);
        assert!(!sess.created_at.to_string().is_empty());
        assert!(sess.current_turn.is_empty());
        assert_eq!(sess.turn_count, 0);
    }

    #[test]
    fn test_create_session_persisted() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();
        assert!(store.meta_path(&sess.id).exists());
    }

    #[test]
    fn test_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();
        let loaded = store.load(&sess.id).unwrap();
        assert_eq!(loaded.id, sess.id);
        assert_eq!(loaded.hash, sess.hash);
    }

    #[test]
    fn test_load_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        assert!(store.load("nonexistent").is_err());
    }

    #[test]
    fn test_save_meta_cost() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let mut sess = store.create().unwrap();
        sess.cost = 0.123456;
        store.save_meta(&sess).unwrap();

        let loaded = store.load(&sess.id).unwrap();
        assert_eq!(loaded.cost, 0.123456);

        let path = store.meta_path(&sess.id);
        let data = std::fs::read_to_string(&path).unwrap();
        assert!(data.contains("\"cost\":0.123456"));
    }

    #[test]
    fn test_save_meta() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let mut sess = store.create().unwrap();
        sess.prompt_tokens = 500;
        sess.completion_tokens = 200;
        sess.current_turn = "a1b2c3d4e5f6a7b8".to_string();
        sess.turn_count = 3;
        store.save_meta(&sess).unwrap();

        let loaded = store.load(&sess.id).unwrap();
        assert_eq!(loaded.prompt_tokens, 500);
        assert_eq!(loaded.completion_tokens, 200);
        assert_eq!(loaded.current_turn, "a1b2c3d4e5f6a7b8");
        assert_eq!(loaded.turn_count, 3);
    }

    #[test]
    fn test_delete() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();
        store.delete(&sess.id).unwrap();
        assert!(!store.session_dir(&sess.id).exists());
    }

    #[test]
    fn test_delete_nonexistent() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        assert!(store.delete("no-such-id").is_ok());
    }

    #[test]
    fn test_list() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess1 = store.create().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let sess2 = store.create().unwrap();

        let sessions = store.list().unwrap();
        assert!(sessions.len() >= 2);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&sess1.id.as_str()));
        assert!(ids.contains(&sess2.id.as_str()));
    }

    #[test]
    fn test_list_skips_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::new(&dir.path().to_string_lossy()).unwrap();
        std::fs::write(store.dir.join("not-a-session.txt"), "data").unwrap();

        let sessions = store.list().unwrap();
        for s in &sessions {
            assert_ne!(s.id, "not-a-session.txt");
        }
    }

    #[test]
    fn test_truncate_multibyte_safe() {
        let s = "日本語のテスト".to_string();
        let out = crate::util::truncate_with_ellipsis(&s, 5, "...");
        assert!(out.ends_with("..."));
        let _ = std::str::from_utf8(out.as_bytes()).unwrap();

        let emoji = "\u{1f980}".repeat(10);
        let out2 = crate::util::truncate_with_ellipsis(&emoji, 4, "...");
        assert!(out2.ends_with("..."));
        let _ = std::str::from_utf8(out2.as_bytes()).unwrap();
    }

    #[test]
    fn test_set_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let mut sess = Session {
            id: "test".to_string(),
            name: String::new(),
            hash: "abc123".to_string(),
            named: false,
            current_turn: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            cost: 0.0,
        };

        store.set_name(&mut sess, "what is go?");
        assert!(sess.named);
        assert!(sess.name.contains("what is go?"));
    }

    #[test]
    fn test_set_name_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let mut sess = Session {
            id: "test".to_string(),
            name: String::new(),
            hash: "abc123".to_string(),
            named: false,
            current_turn: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            cost: 0.0,
        };

        store.set_name(&mut sess, "first message");
        let first_name = sess.name.clone();
        store.set_name(&mut sess, "second message");
        assert_eq!(sess.name, first_name);
    }

    #[test]
    fn test_set_name_multiline_message() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let mut sess = Session {
            id: "test".to_string(),
            name: String::new(),
            hash: "abc123".to_string(),
            named: false,
            current_turn: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            turn_count: 0,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            context_tokens: 0,
            cost: 0.0,
        };

        store.set_name(&mut sess, "line1\nline2\nline3");
        assert!(!sess.name.contains('\n'));
    }

    #[test]
    fn test_commit_turn() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs = vec![
            make_msg(Role::User, "first question"),
            make_msg(Role::Assistant, "first answer"),
        ];
        let turn_id = compute_turn_id("", &msgs);
        let turn = Turn {
            id: turn_id.clone(),
            parent_id: String::new(),
            messages: msgs,
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };

        store.commit_turn(&sess.id, &turn).unwrap();

        assert!(store.turn_path(&sess.id, &turn_id).exists());

        let index = store.turn_index(&sess.id).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].id, turn_id);
        assert_eq!(index[0].parent_id, "");
    }

    #[test]
    fn test_commit_turn_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs = vec![make_msg(Role::User, "hello")];
        let turn_id = compute_turn_id("", &msgs);
        let turn = Turn {
            id: turn_id,
            parent_id: String::new(),
            messages: msgs,
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };

        store.commit_turn(&sess.id, &turn).unwrap();
        assert!(store.commit_turn(&sess.id, &turn).is_err());
    }

    #[test]
    fn test_load_turn() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs = vec![
            make_msg(Role::User, "test question"),
            Message {
                role: Role::Assistant,
                content: "test answer".to_string(),
                reasoning_content: String::new(),
                reasoning_details: vec![],
                tool_calls: vec![crate::message::ToolCall {
                    id: "call_1".to_string(),
                    call_type: "function".to_string(),
                    function: crate::message::ToolCallFunction {
                        name: "bash".to_string(),
                        arguments: r#"{"cmd":"ls"}"#.to_string(),
                    },
                }],
                tool_call_id: String::new(),
                name: String::new(),
                tool_duration: String::new(),
            },
        ];
        let turn_id = compute_turn_id("", &msgs);
        let turn = Turn {
            id: turn_id.clone(),
            parent_id: String::new(),
            messages: msgs.clone(),
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };

        store.commit_turn(&sess.id, &turn).unwrap();
        let loaded = store.load_turn(&sess.id, &turn_id).unwrap();
        assert_eq!(loaded.id, turn_id);
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content, "test question");
        assert_eq!(loaded.messages[1].tool_calls.len(), 1);
    }

    #[test]
    fn test_turn_index_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();
        let index = store.turn_index(&sess.id).unwrap();
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_commit_turn_subagent_in_index() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs = vec![make_msg(Role::User, "sub task")];
        let turn = Turn {
            id: compute_turn_id("parent-1", &msgs),
            parent_id: "parent-1".to_string(),
            messages: msgs,
            created_at: Utc::now(),
            subagent: "sonnet".to_string(),
            tool_call_id: String::new(),
        };

        store.commit_turn(&sess.id, &turn).unwrap();
        let index = store.turn_index(&sess.id).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].subagent, "sonnet");

        let loaded = store.load_turn(&sess.id, &turn.id).unwrap();
        assert_eq!(loaded.subagent, "sonnet");
    }

    #[test]
    fn test_commit_turn_tool_call_id_in_index() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs = vec![make_msg(Role::User, "sub task")];
        let turn = Turn {
            id: compute_turn_id("parent-1", &msgs),
            parent_id: "parent-1".to_string(),
            messages: msgs,
            created_at: Utc::now(),
            subagent: "coder".to_string(),
            tool_call_id: "del_1".to_string(),
        };

        store.commit_turn(&sess.id, &turn).unwrap();
        let index = store.turn_index(&sess.id).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].tool_call_id, "del_1");

        let loaded = store.load_turn(&sess.id, &turn.id).unwrap();
        assert_eq!(loaded.tool_call_id, "del_1");
    }

    #[test]
    fn test_turn_index_cache_invalidation() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let msgs1 = vec![make_msg(Role::User, "first")];
        let turn1_id = compute_turn_id("", &msgs1);
        let turn1 = Turn {
            id: turn1_id,
            parent_id: String::new(),
            messages: msgs1,
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };
        store.commit_turn(&sess.id, &turn1).unwrap();

        let index1 = store.turn_index(&sess.id).unwrap();
        assert_eq!(index1.len(), 1);

        let msgs2 = vec![make_msg(Role::User, "second")];
        let turn2_id = compute_turn_id("", &msgs2);
        let turn2 = Turn {
            id: turn2_id,
            parent_id: String::new(),
            messages: msgs2,
            created_at: Utc::now(),
            subagent: String::new(),
            tool_call_id: String::new(),
        };
        store.commit_turn(&sess.id, &turn2).unwrap();

        let index2 = store.turn_index(&sess.id).unwrap();
        assert_eq!(index2.len(), 2);
    }

    #[test]
    fn test_ancestry_linear() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let root = commit_turn(&mut store, &sess.id, "", "root question", "root answer");
        let child = commit_turn(
            &mut store,
            &sess.id,
            &root.id,
            "child question",
            "child answer",
        );
        let grandchild = commit_turn(
            &mut store,
            &sess.id,
            &child.id,
            "grandchild question",
            "grandchild answer",
        );

        let turns = store.ancestry(&sess.id, &grandchild.id).unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].id, root.id);
        assert_eq!(turns[1].id, child.id);
        assert_eq!(turns[2].id, grandchild.id);
    }

    #[test]
    fn test_ancestry_single_turn() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let root = commit_turn(&mut store, &sess.id, "", "only", "answer");

        let turns = store.ancestry(&sess.id, &root.id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, root.id);
    }

    #[test]
    fn test_ancestry_branching() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        let root = commit_turn(&mut store, &sess.id, "", "plan", "ok");
        let branch_a = commit_turn(&mut store, &sess.id, &root.id, "implement A", "done");
        let branch_b = commit_turn(&mut store, &sess.id, &root.id, "implement B", "done");

        let turns_a = store.ancestry(&sess.id, &branch_a.id).unwrap();
        assert_eq!(turns_a.len(), 2);
        assert_eq!(turns_a[0].id, root.id);
        assert_eq!(turns_a[1].id, branch_a.id);

        let turns_b = store.ancestry(&sess.id, &branch_b.id).unwrap();
        assert_eq!(turns_b.len(), 2);
        assert_eq!(turns_b[0].id, root.id);
        assert_eq!(turns_b[1].id, branch_b.id);
    }

    #[test]
    fn test_delete_cleans_index_cache() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();
        let sess = store.create().unwrap();

        commit_turn(&mut store, &sess.id, "", "hello", "hi");
        store.turn_index(&sess.id).unwrap();

        store.delete(&sess.id).unwrap();

        let mut store2 = Store::new(&store.dir()).unwrap();
        let sess2 = store2.create().unwrap();
        let index = store2.turn_index(&sess2.id).unwrap();
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(
            crate::util::truncate_with_ellipsis("hello", 10, "..."),
            "hello"
        );
        assert_eq!(
            crate::util::truncate_with_ellipsis("hello world", 8, "..."),
            "hello..."
        );
        assert_eq!(crate::util::truncate_with_ellipsis("abc", 3, "..."), "abc");
    }

    #[test]
    fn test_random_hash() {
        let h1 = random_hash();
        let h2 = random_hash();
        assert_eq!(h1.len(), 6);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_turn_index_lru_eviction() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut store = Store::new(&dir.path().to_string_lossy()).unwrap();

        let mut first_id = String::new();
        for i in 0..65 {
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let sess = store.create().unwrap();
            if i == 0 {
                first_id = sess.id.clone();
            }
            let msgs = vec![make_msg(Role::User, "msg")];
            let turn = Turn {
                id: compute_turn_id("", &msgs),
                parent_id: String::new(),
                messages: msgs,
                created_at: Utc::now(),
                subagent: String::new(),
                tool_call_id: String::new(),
            };
            store.commit_turn(&sess.id, &turn).unwrap();
            store.turn_index(&sess.id).unwrap();
        }

        let in_cache = store.index_cache.contains_key(&first_id);
        assert!(!in_cache, "first session should have been evicted");
        assert!(store.index_cache.len() <= 64);

        let index = store.turn_index(&first_id).unwrap();
        assert_eq!(index.len(), 1);
    }
}
