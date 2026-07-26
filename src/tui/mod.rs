pub mod chat;
pub mod chat_dropdowns;
pub mod chat_format;
pub mod chat_handlers;
pub mod chat_render;
pub mod config_editor;
pub mod file_picker;
pub mod session_list;
pub mod theme;

use crate::agent::Event;
use crate::core::Deps;
use crate::message::Message;
use crate::session::Session;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event as CrosstermEvent, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chat::{ChatScreen, Modal};
use chat_render::{render_shortcuts_bar, render_top_bar};
use config_editor::ConfigScreen;
use session_list::SessionListScreen;

enum ConfirmKind {
    DeleteSession(Session),
    DeleteConfigItem,
    DiscardConfig,
    QuitUnsaved,
    AbortResponse,
}

enum AppState {
    SessionList,
    Chat,
    Config,
    Confirm(ConfirmKind),
}

pub struct App {
    state: AppState,
    deps: Deps,
    session_list: SessionListScreen,
    chat: ChatScreen,
    config_screen: ConfigScreen,
    should_quit: bool,
}

impl App {
    pub fn new(deps: Deps) -> Self {
        let mut chat = ChatScreen::new(
            deps.cwd.clone(),
            deps.templates.clone(),
            deps.config.tui.show_thinking,
            deps.config.tui.tools_expanded,
            deps.config.tui.show_subagent_calls,
        );
        chat.start_file_reindex();
        let config_screen = ConfigScreen::new(deps.config_dir.clone());
        App {
            state: AppState::SessionList,
            deps,
            session_list: SessionListScreen::new(),
            chat,
            config_screen,
            should_quit: false,
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let _enter = runtime.enter();

        let mut terminal = ratatui::init();
        execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
        terminal.clear()?;

        crate::herdr::report(crate::herdr::State::Idle);
        self.session_list.load(&mut self.deps.store);

        while !self.should_quit {
            self.chat.poll_files_index();
            terminal.draw(|f| self.draw(f))?;

            if self.chat.waiting {
                let mut pending_events = Vec::new();
                if let Some(ref mut events) = self.chat.events {
                    loop {
                        match events.try_recv() {
                            Ok(event) => {
                                pending_events.push(event);
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                                break;
                            }
                        }
                    }
                }
                for event in pending_events {
                    self.handle_agent_event(event);
                }
                if (self.chat.events.is_none()
                    || self
                        .chat
                        .events
                        .as_ref()
                        .map(|r| r.is_closed())
                        .unwrap_or(true))
                    && self.chat.waiting
                {
                    self.chat.finish_bot_message_now();
                    self.chat.waiting = false;
                }
                self.chat.wait_ticks += 1;
            }

            if event::poll(Duration::from_millis(100))? {
                let ev = event::read()?;
                self.handle_input(ev);
            }
        }

        crate::herdr::shutdown();

        execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture
        )?;
        ratatui::restore();
        Ok(())
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let area = f.area();
        match self.state {
            AppState::SessionList => {
                render_top_bar(
                    f,
                    ratatui::layout::Rect::new(area.x, area.y, area.width, 1),
                    &self.deps.cwd,
                    "",
                );
                let list_area = ratatui::layout::Rect::new(
                    area.x,
                    area.y + 2,
                    area.width,
                    area.height.saturating_sub(4),
                );
                self.session_list.render(f, list_area);
                let shortcuts_area = ratatui::layout::Rect::new(
                    area.x,
                    area.height.saturating_sub(1),
                    area.width,
                    1,
                );
                render_shortcuts_bar(
                    f,
                    shortcuts_area,
                    &[
                        ("↑↓", "navigate"),
                        ("enter", "select"),
                        ("n", "new"),
                        ("d", "delete"),
                        ("q", "quit"),
                    ],
                );
            }
            AppState::Chat => {
                self.chat.render(f);
            }
            AppState::Config => {
                self.config_screen.render(f);
            }
            AppState::Confirm(ref kind) => {
                let msg = match kind {
                    ConfirmKind::DeleteSession(s) => format!("Delete \"{}\"?", s.name),
                    ConfirmKind::DeleteConfigItem => "Delete this item?".to_string(),
                    ConfirmKind::DiscardConfig => "Discard unsaved config changes?".to_string(),
                    ConfirmKind::QuitUnsaved => {
                        "Quit and discard the running response?".to_string()
                    }
                    ConfirmKind::AbortResponse => "Abort the running response?".to_string(),
                };
                let dialog = format!("{}\n\n[y] yes  [n] no  [esc] cancel", msg);
                let p = ratatui::widgets::Paragraph::new(dialog)
                    .style(ratatui::style::Style::default().fg(crate::tui::theme::COLORS.error))
                    .block(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .border_style(
                                ratatui::style::Style::default()
                                    .fg(crate::tui::theme::COLORS.error),
                            ),
                    )
                    .alignment(ratatui::layout::Alignment::Center);

                let dialog_width = 40u16;
                let dialog_height = 5u16;
                let x = (area.width.saturating_sub(dialog_width)) / 2;
                let y = (area.height.saturating_sub(dialog_height)) / 2;
                let dialog_area = ratatui::layout::Rect::new(x, y, dialog_width, dialog_height);
                f.render_widget(p, dialog_area);
            }
        }
    }

    fn handle_input(&mut self, ev: CrosstermEvent) {
        match ev {
            CrosstermEvent::Key(key) if key.kind == KeyEventKind::Press => {
                let code = key.code;
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let alt = key.modifiers.contains(KeyModifiers::ALT);

                match self.state {
                    AppState::Config => {
                        if let Some(true) = self.config_screen.handle_key(key) {
                            self.state = AppState::SessionList;
                        } else if self.config_screen.take_pending_delete() {
                            self.state = AppState::Confirm(ConfirmKind::DeleteConfigItem);
                        } else if self.config_screen.take_pending_close() {
                            self.state = AppState::Confirm(ConfirmKind::DiscardConfig);
                        }
                    }
                    AppState::Confirm(_) => match code {
                        KeyCode::Char('y') => {
                            if let AppState::Confirm(ref k) = self.state {
                                match k {
                                    ConfirmKind::DeleteSession(sess) => {
                                        let sid = sess.id.clone();
                                        let _ = self.deps.store.delete(&sid);
                                        self.session_list.load(&mut self.deps.store);
                                        self.state = AppState::SessionList;
                                    }
                                    ConfirmKind::DeleteConfigItem => {
                                        self.config_screen.confirm_delete_item();
                                        self.state = AppState::Config;
                                    }
                                    ConfirmKind::DiscardConfig => {
                                        self.config_screen.reload();
                                        self.state = AppState::SessionList;
                                    }
                                    ConfirmKind::QuitUnsaved => {
                                        self.should_quit = true;
                                    }
                                    ConfirmKind::AbortResponse => {
                                        self.do_abort_response();
                                        self.state = AppState::Chat;
                                    }
                                }
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            if let AppState::Confirm(ref k) = self.state {
                                match k {
                                    ConfirmKind::DeleteSession(_) => {
                                        self.state = AppState::SessionList;
                                    }
                                    ConfirmKind::DeleteConfigItem | ConfirmKind::DiscardConfig => {
                                        self.state = AppState::Config;
                                    }
                                    ConfirmKind::QuitUnsaved | ConfirmKind::AbortResponse => {
                                        self.state = AppState::Chat;
                                    }
                                }
                            }
                        }
                        _ => {}
                    },
                    AppState::SessionList => match code {
                        KeyCode::Char('q') => {
                            self.should_quit = true;
                        }
                        KeyCode::Char('n') => {
                            if let Ok(sess) = self.deps.store.create() {
                                self.switch_to_session(sess);
                            }
                        }
                        KeyCode::Up => {
                            self.session_list.select_up();
                        }
                        KeyCode::Down => {
                            self.session_list.select_down();
                        }
                        KeyCode::Enter => {
                            if self.session_list.is_new_selected() {
                                if let Ok(sess) = self.deps.store.create() {
                                    self.switch_to_session(sess);
                                }
                            } else if let Some(sess) = self.session_list.selected_session()
                                && let Ok(sess) = self.deps.store.load(&sess.id.clone())
                            {
                                self.switch_to_session(sess);
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(session) = self.session_list.selected_session() {
                                self.state =
                                    AppState::Confirm(ConfirmKind::DeleteSession(session.clone()));
                            }
                        }
                        _ => {}
                    },
                    AppState::Chat => {
                        self.chat.blink_on = true;
                        self.chat.blink_at = Instant::now();
                        match (ctrl, alt, code) {
                            (true, false, KeyCode::Char('c')) => {
                                if !self.chat.textarea.is_empty() {
                                    self.chat.clear_textarea();
                                    self.chat.ctrl_c_armed_at = Some(Instant::now());
                                    return;
                                }
                                let armed = self
                                    .chat
                                    .ctrl_c_armed_at
                                    .map(|t| t.elapsed() < Duration::from_millis(1500))
                                    .unwrap_or(false);
                                if armed {
                                    self.request_quit();
                                } else {
                                    self.chat.ctrl_c_armed_at = Some(Instant::now());
                                }
                            }
                            (false, false, KeyCode::Esc) => {
                                if self.chat.active_modal != Modal::None {
                                    self.chat.close_dropdowns();
                                    return;
                                }
                                if self.chat.waiting {
                                    self.state = AppState::Confirm(ConfirmKind::AbortResponse);
                                    return;
                                }
                                self.reload_current_session();
                                self.state = AppState::SessionList;
                                self.chat.reset();
                                self.session_list.load(&mut self.deps.store);
                            }
                            (false, false, KeyCode::Enter)
                                if self.chat.active_modal == Modal::Command =>
                            {
                                if let Some(item) = self.chat.command_dropdown.selected_item() {
                                    let action = item.value.clone();
                                    self.chat.close_dropdowns();
                                    self.execute_command(&action);
                                }
                            }
                            (false, false, KeyCode::Enter)
                                if self.chat.active_modal == Modal::Template =>
                            {
                                if let Some(item) = self.chat.template_dropdown.selected_item() {
                                    let name = item.template.name.clone();
                                    self.chat.close_dropdowns();
                                    self.chat.complete_prefix('/', &format!("/{} ", name));
                                }
                            }
                            (false, false, KeyCode::Enter)
                                if self.chat.active_modal == Modal::File =>
                            {
                                if let Some(item) = self.chat.file_dropdown.selected_item() {
                                    let path = item.label.clone();
                                    self.chat.close_dropdowns();
                                    self.chat.complete_prefix('@', &format!("@{} ", path));
                                }
                            }
                            (false, false, KeyCode::Enter)
                                if self.chat.active_modal == Modal::Model =>
                            {
                                if let Some(item) = self.chat.model_dropdown.selected_item() {
                                    let id = item.value.clone();
                                    self.chat.close_dropdowns();
                                    self.switch_model(&id);
                                }
                            }
                            (true, false, KeyCode::Char('p')) => {
                                self.chat.open_command_dropdown();
                            }
                            (false, false, KeyCode::Char('/'))
                                if self.chat.active_modal == Modal::None =>
                            {
                                self.chat.insert_char('/');
                                self.chat.open_template_dropdown("");
                            }
                            (false, false, KeyCode::Char('@'))
                                if self.chat.active_modal == Modal::None =>
                            {
                                self.chat.insert_char('@');
                                self.chat.ensure_files_loaded();
                                self.chat.open_file_dropdown("");
                            }
                            (true, false, KeyCode::Char('r')) => {
                                if self.chat.retry_available
                                    && let Some(ref asession) = self.chat.active_session
                                {
                                    match asession.retry() {
                                        Ok(events) => {
                                            self.chat.events = Some(events);
                                            self.chat.waiting = true;
                                            self.chat.wait_start = Instant::now();
                                            self.chat.wait_ticks = 0;
                                            self.chat.retry_available = false;
                                            self.chat.user_scrolled_up = false;
                                            self.chat.scroll_to_bottom();
                                        }
                                        Err(e) => {
                                            self.chat.add_message("error", &e);
                                        }
                                    }
                                }
                            }
                            (false, true, KeyCode::Enter) | (true, false, KeyCode::Char('j')) => {
                                if !self.chat.textarea.is_empty() {
                                    self.submit_prompt();
                                }
                            }
                            (false, false, KeyCode::Tab)
                                if self.chat.active_modal == Modal::Template =>
                            {
                                if let Some(item) = self.chat.template_dropdown.selected_item() {
                                    let name = item.template.name.clone();
                                    self.chat.close_dropdowns();
                                    self.chat.complete_prefix('/', &format!("/{} ", name));
                                }
                            }
                            (false, false, KeyCode::Tab)
                                if self.chat.active_modal == Modal::File =>
                            {
                                if let Some(item) = self.chat.file_dropdown.selected_item() {
                                    let path = item.label.clone();
                                    self.chat.close_dropdowns();
                                    self.chat.complete_prefix('@', &format!("@{} ", path));
                                }
                            }
                            (true, false, KeyCode::Left) => self.chat.cursor_left_word(),
                            (true, false, KeyCode::Right) => self.chat.cursor_right_word(),
                            _ => {
                                if self.chat.waiting {
                                    return;
                                }
                                if self.chat.active_modal != Modal::None {
                                    self.handle_modal_key(code);
                                    return;
                                }
                                match code {
                                    KeyCode::Up => {
                                        if !self.chat.cursor_up()
                                            && self.chat.history_idx + 1
                                                < self.chat.history.len() as isize
                                        {
                                            self.chat.history_idx += 1;
                                            let idx =
                                                self.chat.history.len().saturating_sub(
                                                    1 + self.chat.history_idx as usize,
                                                );
                                            self.chat.set_text(&self.chat.history[idx].clone());
                                        }
                                    }
                                    KeyCode::Down => {
                                        if !self.chat.cursor_down() && self.chat.history_idx >= 0 {
                                            self.chat.history_idx -= 1;
                                            if self.chat.history_idx >= 0 {
                                                let idx = self.chat.history.len().saturating_sub(
                                                    1 + self.chat.history_idx as usize,
                                                );
                                                self.chat.set_text(&self.chat.history[idx].clone());
                                            } else {
                                                self.chat.clear_textarea();
                                            }
                                        }
                                    }
                                    KeyCode::Left => self.chat.cursor_left(),
                                    KeyCode::Right => self.chat.cursor_right(),
                                    KeyCode::Home => self.chat.cursor_home(),
                                    KeyCode::End => self.chat.cursor_end(),
                                    KeyCode::Backspace => self.chat.delete_before_cursor(),
                                    KeyCode::Delete => self.chat.delete_after_cursor(),
                                    KeyCode::Enter => self.chat.insert_char('\n'),
                                    KeyCode::Char(c) => self.chat.insert_char(c),
                                    KeyCode::PageUp => {
                                        self.chat.viewport_offset =
                                            self.chat.viewport_offset.saturating_sub(10);
                                        self.chat.user_scrolled_up = true;
                                    }
                                    KeyCode::PageDown => {
                                        self.chat.viewport_offset += 10;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            CrosstermEvent::Resize(_, _) => {}
            CrosstermEvent::Mouse(mouse) => {
                if matches!(self.state, AppState::Chat) {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            self.chat.viewport_offset = self.chat.viewport_offset.saturating_sub(3);
                            self.chat.user_scrolled_up = true;
                        }
                        MouseEventKind::ScrollDown => {
                            self.chat.viewport_offset += 3;
                        }
                        _ => {}
                    }
                }
            }
            CrosstermEvent::Paste(s)
                if matches!(self.state, AppState::Chat)
                    && !self.chat.waiting
                    && self.chat.active_modal == Modal::None =>
            {
                self.chat.insert_str(&s);
            }
            _ => {}
        }
    }

    fn handle_modal_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => match self.chat.active_modal {
                Modal::Template => self.chat.template_dropdown.up(),
                Modal::File => self.chat.file_dropdown.up(),
                Modal::Command => self.chat.command_dropdown.up(),
                Modal::Model => self.chat.model_dropdown.up(),
                Modal::Tree => self.chat.tree_dropdown.up(),
                _ => {}
            },
            KeyCode::Down => match self.chat.active_modal {
                Modal::Template => self.chat.template_dropdown.down(),
                Modal::File => self.chat.file_dropdown.down(),
                Modal::Command => self.chat.command_dropdown.down(),
                Modal::Model => self.chat.model_dropdown.down(),
                Modal::Tree => self.chat.tree_dropdown.down(),
                _ => {}
            },
            KeyCode::Esc => {
                self.chat.close_dropdowns();
            }
            KeyCode::Char(c) => {
                if self.chat.active_modal == Modal::Template {
                    self.chat.insert_char(c);
                    let query = self.chat.textarea_value().to_string();
                    self.chat.filter_template_dropdown(&query);
                } else if self.chat.active_modal == Modal::File {
                    self.chat.insert_char(c);
                    let query = self.chat.textarea_value().to_string();
                    self.chat.filter_file_dropdown(&query);
                } else if self.chat.active_modal == Modal::Command {
                    self.chat.command_query.push(c);
                    self.chat.command_dropdown.selected = 0;
                    self.chat.filter_command_dropdown();
                }
            }
            KeyCode::Backspace => {
                if self.chat.active_modal == Modal::Template {
                    self.chat.delete_before_cursor();
                    let query = self.chat.textarea_value().to_string();
                    self.chat.filter_template_dropdown(&query);
                } else if self.chat.active_modal == Modal::File {
                    self.chat.delete_before_cursor();
                    let query = self.chat.textarea_value().to_string();
                    self.chat.filter_file_dropdown(&query);
                } else if self.chat.active_modal == Modal::Command {
                    self.chat.command_query.pop();
                    self.chat.filter_command_dropdown();
                }
            }
            _ => {}
        }
    }

    fn do_abort_response(&mut self) {
        if let Some(h) = self.chat.abort_handle.take() {
            h.abort();
        }
        self.chat.events = None;
        self.chat.finish_bot_message_now();
        if let Some(last) = self.chat.messages.last_mut()
            && last.role == "assistant"
        {
            last.stopped = true;
            last.rendered.clear();
        }
        self.chat.waiting = false;
    }

    fn request_quit(&mut self) {
        if self.chat.waiting {
            self.state = AppState::Confirm(ConfirmKind::QuitUnsaved);
        } else {
            self.should_quit = true;
        }
    }

    fn submit_prompt(&mut self) {
        let text = self.chat.textarea_value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.chat.history.push(text.clone());
        self.chat.clear_textarea();
        let expanded = crate::prompts::expand_text(&self.chat.templates, &text);
        self.chat.add_message("user", &expanded);

        if self.chat.active_session.is_none() {
            let sess = match self.deps.store.create() {
                Ok(s) => s,
                Err(e) => {
                    self.chat
                        .add_message("error", &format!("Failed to create session: {}", e));
                    return;
                }
            };
            let asession = self.deps.new_session(sess.clone());
            self.chat.session_name = sess.hash;
            self.chat.model_name = self.deps.model_name.clone();
            self.chat.context_window = asession.context_window();
            self.chat.total_tokens = asession.context_tokens();
            self.chat.total_cost = 0.0;
            self.chat.active_session = Some(asession);
        }

        let (events, handle) = {
            if let Some(ref mut asession) = self.chat.active_session {
                asession.prompt_with_handle(&expanded)
            } else {
                return;
            }
        };

        self.chat.events = Some(events);
        self.chat.abort_handle = Some(handle);
        self.chat.waiting = true;
        self.chat.wait_start = Instant::now();
        self.chat.wait_ticks = 0;
        self.chat.retry_available = false;
        self.chat.user_scrolled_up = false;
        self.chat.scroll_to_bottom();
    }

    fn handle_agent_event(&mut self, event: Event) {
        if matches!(&event.kind, crate::agent::EventKind::AgentDone(_)) && event.subagent.is_empty()
        {
            self.chat.finish_bot_message_now();
            self.reload_current_session();
            return;
        }
        if matches!(&event.kind, crate::agent::EventKind::Error(_)) && event.subagent.is_empty() {
            self.chat.handle_agent_event_inner(&event);
            self.reload_current_session();
            return;
        }
        self.chat.handle_agent_event_inner(&event);
    }

    fn reload_current_session(&mut self) {
        if let Some(ref mut asession) = self.chat.active_session {
            let id = asession.sess().id;
            if let Ok(sess) = self.deps.store.load(&id) {
                self.chat.total_cost = sess.cost;
                asession.reload_from(sess);
                self.chat.total_tokens = asession.context_tokens();
            }
        }
    }

    fn switch_to_session(&mut self, sess: Session) {
        let asession = self.deps.new_session(sess.clone());
        self.chat.reset();

        if !sess.current_turn.is_empty()
            && let Ok(turns) = self.deps.store.ancestry(&sess.id, &sess.current_turn)
        {
            let ancestry_ids: std::collections::HashSet<String> =
                turns.iter().map(|t| t.id.clone()).collect();

            let mut msgs = Vec::new();
            let mut children: std::collections::HashMap<String, Vec<Message>> =
                std::collections::HashMap::new();

            for t in &turns {
                msgs.extend_from_slice(&t.messages);
            }

            if let Ok(index) = self.deps.store.turn_index(&sess.id) {
                for m in &index {
                    if !m.tool_call_id.is_empty()
                        && ancestry_ids.contains(&m.parent_id)
                        && let Ok(sub_turn) = self.deps.store.load_turn(&sess.id, &m.id)
                    {
                        children
                            .entry(m.tool_call_id.clone())
                            .or_default()
                            .extend(sub_turn.messages);
                    }
                }
            }

            self.chat.load_messages(&msgs, &children);
        }

        self.chat.active_session = Some(asession);
        self.chat.session_name = sess.hash.clone();
        self.chat.model_name = self.deps.model_name.clone();
        self.chat.context_window = self.chat.active_session.as_ref().unwrap().context_window();
        self.chat.total_tokens = self.chat.active_session.as_ref().unwrap().context_tokens();
        self.chat.total_cost = sess.cost;
        self.chat.scroll_to_bottom();
        self.state = AppState::Chat;
    }

    fn execute_command(&mut self, action: &str) {
        match action {
            "new" => {
                self.reload_current_session();
                self.state = AppState::SessionList;
                self.chat.reset();
                if let Ok(sess) = self.deps.store.create() {
                    self.switch_to_session(sess);
                }
            }
            "back" => {
                self.reload_current_session();
                self.state = AppState::SessionList;
                self.chat.reset();
                self.session_list.load(&mut self.deps.store);
            }
            "tools" => {
                self.chat.tools_expanded = !self.chat.tools_expanded;
                self.chat.rerender_all();
                let _ = crate::config::save_tui(
                    &self.deps.config_dir,
                    self.chat.tools_expanded,
                    self.chat.show_thinking,
                    self.chat.show_subagent_calls,
                );
            }
            "subagents" => {
                self.chat.show_subagent_calls = !self.chat.show_subagent_calls;
                self.chat.rerender_all();
                let _ = crate::config::save_tui(
                    &self.deps.config_dir,
                    self.chat.tools_expanded,
                    self.chat.show_thinking,
                    self.chat.show_subagent_calls,
                );
            }
            "thinking" => {
                self.chat.show_thinking = !self.chat.show_thinking;
                self.chat.rerender_all();
                let _ = crate::config::save_tui(
                    &self.deps.config_dir,
                    self.chat.tools_expanded,
                    self.chat.show_thinking,
                    self.chat.show_subagent_calls,
                );
            }
            "copy-last" => {
                self.chat.copy_last_response();
            }
            "export-md" => {
                self.chat.export_markdown();
            }
            "tree" => {
                if let Some(ref asession) = self.chat.active_session {
                    let sid = asession.sess().id;
                    if let Ok(index) = self.deps.store.turn_index(&sid) {
                        self.chat
                            .build_tree_from_index(&index, &asession.sess().current_turn);
                        self.chat.show_tree();
                    }
                }
            }
            "config" => {
                self.config_screen.reload();
                self.state = AppState::Config;
            }
            "model" => {
                self.chat.open_model_dropdown(&self.deps.config.models);
            }
            "quit" => {
                self.request_quit();
            }
            _ => {}
        }
    }

    fn switch_model(&mut self, model_id: &str) {
        let verbose = self.deps.client.debug_enabled();
        match crate::core::resolve::resolve_client(
            model_id,
            &self.deps.config.models,
            &self.deps.config.providers,
            verbose,
        ) {
            Ok((client, model_name)) => {
                self.deps.client = client;
                self.deps.model_name = model_name.clone();
                self.chat.model_name = model_name;
                if let Some(ref mut asession) = self.chat.active_session {
                    asession.set_client(Arc::new(self.deps.client.clone()));
                    self.chat.context_window = asession.context_window();
                }
            }
            Err(e) => {
                self.chat.add_message("error", &format!("model: {}", e));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::provider::{Client, ModelProfile};
    use crate::session::store::Store;
    use crate::tools::Registry;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn key(code: KeyCode) -> CrosstermEvent {
        CrosstermEvent::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn app_in_chat() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let deps = Deps {
            agent_name: String::new(),
            client: Client::new("http://localhost", "m", "k", ModelProfile::default()),
            registry: Arc::new(Registry::standard()),
            system_prompt: String::new(),
            cwd: path.clone(),
            store: Store::new(&path).unwrap(),
            subagents: HashMap::new(),
            skills: None,
            config: Config::default_for(&path),
            config_dir: String::new(),
            model_name: String::new(),
            templates: vec![],
        };
        let mut app = App::new(deps);
        app.state = AppState::Chat;
        (app, dir)
    }

    #[test]
    fn slash_in_file_modal_stays_in_file_modal() {
        let (mut app, _dir) = app_in_chat();
        app.handle_input(key(KeyCode::Char('@')));
        assert!(matches!(app.chat.active_modal, Modal::File));

        app.handle_input(key(KeyCode::Char('/')));
        assert!(matches!(app.chat.active_modal, Modal::File));
        assert_eq!(app.chat.textarea, "@/");
    }

    #[test]
    fn at_in_template_modal_stays_in_template_modal() {
        let (mut app, _dir) = app_in_chat();
        app.handle_input(key(KeyCode::Char('/')));
        assert!(matches!(app.chat.active_modal, Modal::Template));

        app.handle_input(key(KeyCode::Char('@')));
        assert!(matches!(app.chat.active_modal, Modal::Template));
        assert_eq!(app.chat.textarea, "/@");
    }
}
