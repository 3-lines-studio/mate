mod edit;
mod fields;
mod rows;

use crate::config::{Config, save_config};
use crate::tui::theme::COLORS;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use edit::Edit;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};
use rows::Row;

pub struct ConfigScreen {
    config: Config,
    dir: String,
    rows: Vec<Row>,
    collapsed: Vec<bool>,
    row: usize,
    scroll: usize,
    view_h: usize,
    edit: Edit,
    dirty: bool,
    msg: String,
    pending_add_section: Option<usize>,
    pub(crate) pending_delete: bool,
    pub(crate) pending_close: bool,
}

impl ConfigScreen {
    pub fn new(dir: String) -> Self {
        let config = crate::config::load_from(&dir).unwrap_or_default();
        let mut s = ConfigScreen {
            config,
            dir,
            rows: Vec::new(),
            collapsed: vec![true; fields::SECTION_NAMES.len()],
            row: 0,
            scroll: 0,
            view_h: 20,
            edit: Edit::None,
            dirty: false,
            msg: String::new(),
            pending_add_section: None,
            pending_delete: false,
            pending_close: false,
        };
        s.rebuild_rows();
        s
    }

    pub fn reload(&mut self) {
        if let Ok(cfg) = crate::config::load_from(&self.dir) {
            self.config = cfg;
        }
        self.row = 0;
        self.scroll = 0;
        self.edit = Edit::None;
        self.dirty = false;
        self.msg = String::new();
        self.rebuild_rows();
    }

    pub fn take_pending_delete(&mut self) -> bool {
        std::mem::take(&mut self.pending_delete)
    }

    pub fn take_pending_close(&mut self) -> bool {
        std::mem::take(&mut self.pending_close)
    }

    pub fn confirm_delete_item(&mut self) {
        self.msg.clear();
        let Some((si, ii)) = self.item_at_cursor() else {
            return;
        };
        match si {
            4 => {
                self.config.providers.remove(ii);
            }
            5 => {
                self.config.models.remove(ii);
            }
            6 => {
                self.config.subagents.remove(ii);
            }
            _ => return,
        }
        self.dirty = true;
        self.rebuild_rows();
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<bool> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if ctrl && key.code == KeyCode::Char('s') {
            self.save();
            return None;
        }

        match &self.edit {
            Edit::Text(_, _) => {
                return self.handle_text_key(key);
            }
            Edit::Pick(_) => {
                return self.handle_pick_key(key);
            }
            Edit::None => {}
        }

        match key.code {
            KeyCode::Up => {
                if self.row > 0 {
                    self.row -= 1;
                    if matches!(self.rows.get(self.row), Some(Row::EmptyHint)) && self.row > 0 {
                        self.row -= 1;
                    }
                    if self.row < self.scroll {
                        self.scroll = self.row;
                    }
                }
            }
            KeyCode::Down => {
                let max = self.rows.len().saturating_sub(1);
                if self.row < max {
                    self.row += 1;
                    if matches!(self.rows.get(self.row), Some(Row::EmptyHint)) && self.row < max {
                        self.row += 1;
                    }
                    if self.row >= self.scroll + self.view_h {
                        self.scroll = self.row + 1 - self.view_h;
                    }
                }
            }
            KeyCode::Enter => self.enter(),
            KeyCode::Esc => {
                if self.dirty {
                    self.pending_close = true;
                    return None;
                }
                self.msg.clear();
                return Some(true);
            }
            KeyCode::Char('a') => self.add_item(),
            KeyCode::Char('d') => self.delete_item(),
            _ => {}
        }
        None
    }

    fn save(&mut self) {
        match save_config(&self.dir, &self.config) {
            Ok(()) => {
                self.dirty = false;
                self.msg = "Saved.".to_string();
            }
            Err(e) => {
                self.msg = format!("Save error: {}", e);
            }
        }
    }

    pub fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let title = if self.dirty {
            "* Config Editor"
        } else {
            "Config Editor"
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLORS.border))
            .title(title)
            .title_alignment(Alignment::Left);
        let hint = "[↑↓] scroll  [⏎] toggle/edit  [a] add  [d] delete  [^S] save  [Esc] close";
        let hint_area = Rect::new(area.x, area.y, area.width, 1);
        let hint_p = Paragraph::new(hint)
            .style(Style::default().fg(COLORS.muted))
            .alignment(Alignment::Right);
        f.render_widget(hint_p, hint_area);

        let content_area = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(3),
        );
        let inner = block.inner(content_area);
        f.render_widget(block, content_area);

        self.view_h = inner.height as usize;

        let items = self.build_render_items();
        let visible = inner.height as usize;
        let end = (self.scroll + visible).min(items.len());
        let slice: Vec<ListItem> = items[self.scroll..end]
            .iter()
            .map(|rl| {
                let style = if rl.selected {
                    Style::default().bg(COLORS.selected).fg(COLORS.accent)
                } else if rl.is_section_hdr {
                    Style::default().fg(COLORS.accent)
                } else {
                    Style::default().fg(COLORS.fg)
                };
                ListItem::new(rl.text.as_str()).style(style)
            })
            .collect();
        let list = List::new(slice);
        f.render_widget(list, inner);

        let status_area = Rect::new(area.x, area.height.saturating_sub(1), area.width, 1);
        let status_line = if !self.msg.is_empty() {
            Paragraph::new(self.msg.as_str()).style(Style::default().fg(COLORS.accent))
        } else {
            Paragraph::new(self.location_hint()).style(Style::default().fg(COLORS.muted))
        };
        f.render_widget(status_line, status_area);

        if let Edit::Text(ref buf, cursor) = self.edit {
            let (rel_x, rel_y) = self.cursor_position(buf, cursor, inner);
            let x = inner.x + rel_x;
            let y = inner.y + rel_y;
            f.set_cursor_position((x, y));
        }

        if let Edit::Pick(ref dd) = self.edit {
            self.render_pick_dropdown(f, dd);
        }
    }
}
