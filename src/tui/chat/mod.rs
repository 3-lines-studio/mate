mod commands;
mod dropdowns;
mod messages;
mod textarea;

use crate::agent::{AgentSession, Event};
use crate::prompts::Template;
use crate::render::StreamRenderer;
use ratatui::{Frame, layout::Rect};
use std::time::{Duration, Instant};

use super::chat_handlers::{ChatMsg, LiveBlock, Segment, finish_bot_message, handle_agent_event};
use super::file_picker::index_files;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Modal {
    None,
    Template,
    File,
    Command,
    Model,
    Tree,
}

pub struct ChatScreen {
    pub messages: Vec<ChatMsg>,
    pub live_blocks: Vec<LiveBlock>,
    pub waiting: bool,
    pub wait_start: Instant,
    pub wait_ticks: usize,
    pub events: Option<tokio::sync::mpsc::Receiver<Event>>,
    pub abort_handle: Option<tokio::task::JoinHandle<()>>,
    pub session_name: String,
    pub model_name: String,
    pub cwd: String,
    pub total_tokens: i32,
    pub cache_hit_tokens: i32,
    pub total_cost: f64,
    pub context_window: i32,
    pub show_thinking: bool,
    pub tools_expanded: bool,
    pub show_subagent_calls: bool,
    pub retry_available: bool,
    pub active_session: Option<AgentSession>,

    pub textarea: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub history_idx: isize,
    pub textarea_height: u16,
    pub textarea_scroll: usize,
    pub blink_at: Instant,
    pub blink_on: bool,

    pub viewport_offset: usize,
    pub viewport_lines: Vec<String>,
    pub user_scrolled_up: bool,
    pub scroll_to_bottom: bool,

    pub active_modal: Modal,
    pub command_dropdown: super::chat_dropdowns::Dropdown<super::chat_dropdowns::LabeledItem>,
    pub command_query: String,
    pub model_dropdown: super::chat_dropdowns::Dropdown<super::chat_dropdowns::LabeledItem>,
    pub template_dropdown: super::chat_dropdowns::Dropdown<super::chat_dropdowns::TemplateItem>,
    pub file_dropdown: super::chat_dropdowns::Dropdown<super::chat_dropdowns::LabeledItem>,
    pub tree_dropdown: super::chat_dropdowns::Dropdown<super::chat_dropdowns::TreeItem>,

    pub all_files: Vec<String>,
    pub files_loaded: bool,
    files_index_rx: Option<std::sync::mpsc::Receiver<Vec<String>>>,
    pub all_template_items: Vec<super::chat_dropdowns::TemplateItem>,
    pub templates: Vec<Template>,
    pub tree_items: Vec<super::chat_dropdowns::TreeItem>,

    pub renderer: StreamRenderer,
    pub width: u16,
    pub height: u16,

    pub ctrl_c_armed_at: Option<Instant>,
    pub finished: bool,
}

impl ChatScreen {
    pub fn new(
        cwd: String,
        templates: Vec<Template>,
        show_thinking: bool,
        tools_expanded: bool,
        show_subagent_calls: bool,
    ) -> Self {
        let tmpl_items: Vec<super::chat_dropdowns::TemplateItem> = templates
            .iter()
            .map(|t| super::chat_dropdowns::TemplateItem {
                template: t.clone(),
                search_key: t.name.clone(),
            })
            .collect();

        ChatScreen {
            messages: Vec::new(),
            live_blocks: Vec::new(),
            waiting: false,
            wait_start: Instant::now(),
            wait_ticks: 0,
            events: None,
            abort_handle: None,
            session_name: String::new(),
            model_name: String::new(),
            cwd,
            total_tokens: 0,
            cache_hit_tokens: 0,
            total_cost: 0.0,
            context_window: 0,
            show_thinking,
            tools_expanded,
            show_subagent_calls,
            retry_available: false,
            active_session: None,

            textarea: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_idx: -1,
            textarea_height: 1,
            textarea_scroll: 0,
            blink_at: Instant::now(),
            blink_on: true,

            viewport_offset: 0,
            viewport_lines: Vec::new(),
            user_scrolled_up: false,
            scroll_to_bottom: false,

            active_modal: Modal::None,
            command_dropdown: super::chat_dropdowns::Dropdown::new(),
            command_query: String::new(),
            model_dropdown: super::chat_dropdowns::Dropdown::new(),
            template_dropdown: super::chat_dropdowns::Dropdown::new(),
            file_dropdown: super::chat_dropdowns::Dropdown::new(),
            tree_dropdown: super::chat_dropdowns::Dropdown::new(),

            all_files: Vec::new(),
            files_loaded: false,
            files_index_rx: None,
            all_template_items: tmpl_items,
            templates,
            tree_items: Vec::new(),

            renderer: StreamRenderer::new(80),
            width: 80,
            height: 24,

            ctrl_c_armed_at: None,
            finished: false,
        }
    }

    pub fn set_size(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
        self.renderer = StreamRenderer::new((w as usize).saturating_sub(5));
        for msg in &mut self.messages {
            msg.rendered.clear();
        }
        for lb in &mut self.live_blocks {
            lb.rendered.clear();
        }
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.live_blocks.clear();
        self.textarea.clear();
        self.cursor = 0;
        self.history.clear();
        self.history_idx = -1;
        self.waiting = false;
        self.active_modal = Modal::None;
        self.retry_available = false;
        self.ctrl_c_armed_at = None;
        self.finished = false;
        self.abort_handle = None;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_to_bottom = true;
        self.user_scrolled_up = false;
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(match role {
            "user" => ChatMsg::user(content),
            "error" => ChatMsg::error(content),
            _ => ChatMsg::assistant(vec![Segment::prose(content)]),
        });
        self.render_messages();
    }

    pub fn handle_agent_event_inner(&mut self, event: &Event) {
        handle_agent_event(
            event,
            &mut self.live_blocks,
            &mut self.messages,
            &self.cwd,
            self.show_thinking,
            self.tools_expanded,
            &mut self.total_tokens,
            &mut self.cache_hit_tokens,
            &mut self.total_cost,
            &mut self.retry_available,
            &mut self.finished,
            self.active_session.as_ref(),
        );
        if self.finished {
            finish_bot_message(&mut self.live_blocks, &mut self.messages, &self.cwd);
            self.waiting = false;
            self.finished = false;
        }
        self.render_messages();
    }

    pub fn finish_bot_message_now(&mut self) {
        self.abort_handle = None;
        finish_bot_message(&mut self.live_blocks, &mut self.messages, &self.cwd);
        self.waiting = false;
        self.start_file_reindex();
        self.render_messages();
    }

    pub fn start_file_reindex(&mut self) {
        let cwd = self.cwd.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(index_files(&cwd));
        });
        self.files_index_rx = Some(rx);
    }

    pub fn poll_files_index(&mut self) {
        let Some(rx) = &self.files_index_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(files) => {
                self.all_files = files;
                self.files_loaded = true;
                self.files_index_rx = None;
                if self.active_modal == Modal::File {
                    let query = self.textarea_value().to_string();
                    self.filter_file_dropdown(&query);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.files_index_rx = None;
            }
        }
    }

    pub fn ensure_files_loaded(&mut self) {
        if self.files_loaded {
            return;
        }
        if self.files_index_rx.is_none() {
            self.start_file_reindex();
        }
        self.poll_files_index();
    }

    pub fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        if self.blink_at.elapsed() >= Duration::from_millis(500) {
            self.blink_on = !self.blink_on;
            self.blink_at = Instant::now();
        }
        if area.width != self.width || area.height != self.height {
            self.set_size(area.width, area.height);
        }
        self.update_textarea_layout();
        let bottom_h = self.bottom_bar_height().min(area.height);

        let msg_area = Rect::new(
            area.x + 2,
            area.y,
            area.width.saturating_sub(4),
            area.height.saturating_sub(bottom_h),
        );
        self.render_messages_area(f, msg_area);
        self.render_bottom_bar(f, area);
    }
}

pub(crate) fn shorten_cwd_string(cwd: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if cwd.starts_with(&*home) {
            return format!("~{}", &cwd[home.len()..]);
        }
    }
    cwd.to_string()
}

pub(super) fn fmt_cost(c: f64) -> String {
    if c < 0.01 {
        format!("${:.4}", c)
    } else {
        format!("${:.2}", c)
    }
}

pub(super) fn fit_height(area: Rect, top: u16, h: u16) -> Option<u16> {
    let bottom = area.y.saturating_add(area.height);
    if top >= bottom {
        return None;
    }
    Some(h.min(bottom - top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn fmt_cost_precision() {
        assert_eq!(fmt_cost(0.0012), "$0.0012");
        assert_eq!(fmt_cost(0.0099), "$0.0099");
        assert_eq!(fmt_cost(0.01), "$0.01");
        assert_eq!(fmt_cost(1.234), "$1.23");
    }

    fn render_to(screen: &mut ChatScreen, w: u16, h: u16) {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| screen.render(f)).unwrap();
    }

    #[test]
    fn bottom_bar_never_overflows_buffer() {
        use std::panic;
        let mut panics: Vec<String> = Vec::new();
        for &h in &[
            1u16, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 24, 30, 40, 50,
            60, 67, 80, 100,
        ] {
            for &(waiting, modal) in &[
                (false, Modal::None),
                (true, Modal::None),
                (false, Modal::Command),
                (true, Modal::Command),
                (false, Modal::Template),
                (true, Modal::Template),
                (false, Modal::File),
                (true, Modal::File),
                (false, Modal::Model),
                (true, Modal::Model),
                (false, Modal::Tree),
            ] {
                for &tall in &[false, true] {
                    let mut s = ChatScreen::new(".".into(), vec![], true, true, true);
                    s.set_size(116, h);
                    for _ in 0..20 {
                        s.add_message("user", &"word ".repeat(60));
                        s.add_message("assistant", &"reply ".repeat(80));
                    }
                    s.active_modal = modal;
                    if modal == Modal::Command {
                        s.open_command_dropdown();
                    }
                    if modal == Modal::Model {
                        s.model_dropdown.items = vec![
                            crate::tui::chat_dropdowns::LabeledItem {
                                label: "gpt-4o".to_string(),
                                value: "gpt-4o".to_string(),
                            },
                            crate::tui::chat_dropdowns::LabeledItem {
                                label: "claude".to_string(),
                                value: "claude-3.5-sonnet".to_string(),
                            },
                        ];
                        s.model_dropdown.selected = 0;
                    }
                    s.waiting = waiting;
                    if tall {
                        s.set_text(&"long input line ".repeat(60));
                    }
                    s.scroll_to_bottom();
                    let res = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                        render_to(&mut s, 116, h);
                    }));
                    if res.is_err() {
                        panics.push(format!(
                            "h={} waiting={} modal={} tall={}",
                            h,
                            waiting,
                            match modal {
                                Modal::None => "None",
                                Modal::Command => "Command",
                                Modal::Template => "Template",
                                Modal::File => "File",
                                Modal::Model => "Model",
                                Modal::Tree => "Tree",
                            },
                            tall
                        ));
                    }
                }
            }
        }
        if !panics.is_empty() {
            for p in &panics {
                eprintln!("PANIC: {}", p);
            }
        }
        assert!(panics.is_empty(), "reproduced {} panics", panics.len());
    }

    #[test]
    fn follows_new_messages_to_bottom_when_not_scrolled() {
        let mut s = ChatScreen::new(".".into(), vec![], true, true, true);
        s.set_size(80, 24);
        for _ in 0..40 {
            s.add_message("user", &"word ".repeat(200));
        }
        render_to(&mut s, 80, 24);
        assert!(!s.user_scrolled_up);
        let max_after = s.viewport_offset;
        assert!(max_after > 0);

        for _ in 0..20 {
            s.add_message("user", &"word ".repeat(200));
        }
        render_to(&mut s, 80, 24);
        assert!(!s.user_scrolled_up);
        assert!(s.viewport_offset > max_after);
    }

    #[test]
    fn stays_pinned_when_user_scrolled_up() {
        let mut s = ChatScreen::new(".".into(), vec![], true, true, true);
        s.set_size(80, 24);
        for _ in 0..40 {
            s.add_message("user", &"word ".repeat(200));
        }
        render_to(&mut s, 80, 24);

        s.user_scrolled_up = true;
        s.viewport_offset = s.viewport_offset.saturating_sub(20);
        let pinned = s.viewport_offset;

        for _ in 0..20 {
            s.add_message("user", &"word ".repeat(200));
        }
        render_to(&mut s, 80, 24);
        assert!(s.user_scrolled_up);
        assert_eq!(s.viewport_offset, pinned);
    }
}
