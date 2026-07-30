use crate::config::{ModelConfig, ProviderConfig, SubagentConfig};
use crate::tui::chat_dropdowns::Dropdown;
use crate::tui::theme::COLORS;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, List, ListItem},
};

use super::fields::{FieldKind, SECTION_NAMES, unicode_display_width};
use super::rows::Row;

pub(super) enum Edit {
    None,
    Text(String, usize),
    Pick(Dropdown<String>),
}

pub(super) struct RenderLine {
    pub(super) text: String,
    pub(super) selected: bool,
    pub(super) is_section_hdr: bool,
}

impl super::ConfigScreen {
    pub(super) fn enter(&mut self) {
        self.msg.clear();
        match self.rows.get(self.row) {
            Some(Row::Section(si)) => {
                self.collapsed[*si] = !self.collapsed[*si];
                self.rebuild_rows();
                return;
            }
            Some(Row::Item(_, _)) => {
                if let Some(pos) = (self.row + 1..self.rows.len())
                    .find(|&i| matches!(self.rows.get(i), Some(Row::Field { .. })))
                {
                    self.row = pos;
                }
                return;
            }
            Some(Row::Field { .. }) => {}
            _ => return,
        }
        if let Some((si, ii, _fi, _name, kind)) = self.field_at_cursor() {
            if kind == FieldKind::Bool {
                self.toggle_bool(si, ii);
                return;
            }
            if matches!(kind, FieldKind::PickModels | FieldKind::PickProviders) {
                let options = self.pick_options(kind);
                let cur = self.cur_value_at();
                let mut dd = Dropdown::new();
                dd.items = options;
                dd.visible = true;
                dd.selected = dd.items.iter().position(|o| o == &cur).unwrap_or(0);
                self.edit = Edit::Pick(dd);
                return;
            }
            let val = self.cur_value_at();
            self.edit = Edit::Text(val.clone(), val.len());
        }
    }

    fn toggle_bool(&mut self, si: usize, ii: Option<usize>) {
        if let Some((_, _, _, name, _)) = self.field_at_cursor() {
            let cur = if let Some(item_idx) = ii {
                self.get_item_field_value(si, item_idx, &name)
            } else {
                self.get_scalar_field_value(si, &name)
            };
            let new_val = if cur == "true" { "false" } else { "true" };
            if let Some(item_idx) = ii {
                let _ = self.set_item_field_value(si, item_idx, &name, new_val);
            } else {
                let _ = self.set_scalar_field_value(si, &name, new_val);
            }
            self.dirty = true;
        }
    }

    pub(super) fn add_item(&mut self) {
        self.msg.clear();
        let Some(si) = self.section_at_cursor() else {
            return;
        };
        if !self.is_array_section(si) {
            return;
        }
        self.edit = Edit::Text(String::new(), 0);
        self.pending_add_section = Some(si);
    }

    pub(super) fn delete_item(&mut self) {
        self.msg.clear();
        if self.item_at_cursor().is_some() {
            self.pending_delete = true;
        }
    }

    pub(super) fn handle_text_key(&mut self, key: KeyEvent) -> Option<bool> {
        if let Edit::Text(buf, cursor) = &self.edit {
            let mut buf = buf.clone();
            let mut cursor = *cursor;
            match key.code {
                KeyCode::Esc => {
                    self.edit = Edit::None;
                }
                KeyCode::Enter => {
                    self.edit = Edit::None;
                    self.apply_text(&buf);
                }
                KeyCode::Backspace => {
                    if cursor > 0 {
                        let prev = buf[..cursor].chars().next_back().unwrap().len_utf8();
                        cursor -= prev;
                        buf.remove(cursor);
                    }
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::Delete => {
                    if cursor < buf.len() {
                        buf.remove(cursor);
                    }
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::Left => {
                    if cursor > 0 {
                        let prev = buf[..cursor].chars().next_back().unwrap().len_utf8();
                        cursor -= prev;
                    }
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::Right => {
                    if cursor < buf.len() {
                        let next = buf[cursor..].chars().next().unwrap().len_utf8();
                        cursor += next;
                    }
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::Home => {
                    cursor = 0;
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::End => {
                    cursor = buf.len();
                    self.edit = Edit::Text(buf, cursor);
                }
                KeyCode::Char(c) => {
                    buf.insert(cursor, c);
                    cursor += c.len_utf8();
                    self.edit = Edit::Text(buf, cursor);
                }
                _ => {}
            }
        }
        None
    }

    pub(super) fn handle_pick_key(&mut self, key: KeyEvent) -> Option<bool> {
        if let Edit::Pick(dd) = &self.edit {
            let mut dd = dd.clone();
            match key.code {
                KeyCode::Esc => {
                    self.edit = Edit::None;
                }
                KeyCode::Enter => {
                    let sel = dd.selected_item().cloned();
                    self.edit = Edit::None;
                    if let Some(val) = sel {
                        self.apply_text(&val);
                    }
                }
                KeyCode::Up => {
                    dd.up();
                    self.edit = Edit::Pick(dd);
                }
                KeyCode::Down => {
                    dd.down();
                    self.edit = Edit::Pick(dd);
                }
                _ => {}
            }
        }
        None
    }

    fn apply_text(&mut self, value: &str) {
        if let Some(si) = self.pending_add_section.take() {
            self.collapsed[si] = false;
            self.do_add_item(si, value);
            self.rebuild_rows();
            if let Some(pos) = self
                .rows
                .iter()
                .rposition(|r| matches!(r, Row::Item(s, _) if *s == si))
            {
                self.row = pos;
            }
            return;
        }
        if let Some((si, ii, _fi, name, _kind)) = self.field_at_cursor() {
            let res = if let Some(item_idx) = ii {
                self.set_item_field_value(si, item_idx, &name, value)
            } else {
                self.set_scalar_field_value(si, &name, value)
            };
            match res {
                Ok(()) => self.dirty = true,
                Err(e) => self.msg = e,
            }
        }
    }

    fn do_add_item(&mut self, section_idx: usize, id: &str) {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return;
        }
        match section_idx {
            4 => {
                self.config.providers.push(ProviderConfig {
                    id: trimmed.to_string(),
                    base_url: String::new(),
                    open_router: false,
                    api_key: String::new(),
                });
            }
            5 => {
                self.config.models.push(ModelConfig {
                    id: trimmed.to_string(),
                    name: trimmed.to_string(),
                    provider: String::new(),
                    context_window: 0,
                    max_output_tokens: 0,
                    reasoning_effort: String::new(),
                    input_price: 0.0,
                    cached_input_price: 0.0,
                    output_price: 0.0,
                    prompt_cache: false,
                    cache_ttl: String::new(),
                });
            }
            6 => {
                self.config.subagents.push(SubagentConfig {
                    id: trimmed.to_string(),
                    description: String::new(),
                    model: String::new(),
                    tools: Vec::new(),
                    prompt: String::new(),
                });
            }
            _ => {}
        }
        self.dirty = true;
    }

    pub(super) fn cursor_position(&self, buf: &str, cursor: usize, list_area: Rect) -> (u16, u16) {
        let items = self.build_render_items();
        let visible = list_area.height as usize;
        let end = (self.scroll + visible).min(items.len());
        let slice = &items[self.scroll..end];
        let row_offset = slice.iter().position(|rl| rl.selected).unwrap_or(0);
        let indent = match self.field_at_cursor() {
            Some((_, ii, _, _, _)) if ii.is_some() => "    ",
            _ => "  ",
        };
        let name = self
            .field_at_cursor()
            .map(|(_, _, _, n, _)| n)
            .unwrap_or_default();
        let prefix = format!("{}{}: ", indent, name);
        let prefix_len = unicode_display_width(&prefix);
        let text_before = &buf[..cursor];
        let col = prefix_len + unicode_display_width(text_before);
        (
            (col as u16).min(list_area.width.saturating_sub(1)),
            row_offset as u16,
        )
    }

    pub(super) fn render_pick_dropdown(&self, f: &mut Frame, dd: &Dropdown<String>) {
        let max_width = dd.items.iter().map(|s| s.len()).max().unwrap_or(0) as u16 + 4;
        let height = (dd.items.len() as u16).min(10) + 2;
        let popup_w = max_width.max(16);
        let popup_h = height;
        let area = f.area();
        let x = (area.width.saturating_sub(popup_w)) / 2;
        let y = (area.height.saturating_sub(popup_h)) / 2;
        let popup = Rect::new(x, y, popup_w, popup_h);
        f.render_widget(Clear, popup);
        let items: Vec<ListItem> = dd
            .items
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i == dd.selected {
                    ListItem::new(format!(" {}", s))
                        .style(Style::default().bg(COLORS.selected).fg(COLORS.accent))
                } else {
                    ListItem::new(format!(" {}", s)).style(Style::default().fg(COLORS.muted))
                }
            })
            .collect();
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLORS.border)),
        );
        f.render_widget(list, popup);
    }

    pub(super) fn location_hint(&self) -> &str {
        self.section_at_cursor()
            .and_then(|si| SECTION_NAMES.get(si).copied())
            .unwrap_or("")
    }

    pub(super) fn build_render_items(&self) -> Vec<RenderLine> {
        let mut items = Vec::new();
        let edit_buf = match &self.edit {
            Edit::Text(buf, _) => Some(buf.as_str()),
            _ => None,
        };
        for (i, row) in self.rows.iter().enumerate() {
            match row {
                Row::Section(si) => {
                    let name = SECTION_NAMES[*si];
                    let marker = if self.collapsed[*si] { '▸' } else { '▾' };
                    let extra = if self.is_array_section(*si) {
                        format!(" [{}]", self.array_len(*si))
                    } else {
                        String::new()
                    };
                    items.push(RenderLine {
                        text: format!("{} {}{}", marker, name, extra),
                        selected: i == self.row,
                        is_section_hdr: true,
                    });
                }
                Row::Item(si, ii) => {
                    let id = match si {
                        4 => self.config.providers[*ii].id.clone(),
                        5 => self.config.models[*ii].id.clone(),
                        6 => self.config.subagents[*ii].id.clone(),
                        _ => String::new(),
                    };
                    items.push(RenderLine {
                        text: format!("  ▸ {}", id),
                        selected: i == self.row,
                        is_section_hdr: false,
                    });
                }
                Row::Field { si, ii, fi } => {
                    let fields = self.fields_for_section(*si);
                    let &(name, _kind) = fields.get(*fi).unwrap();
                    let indent = if ii.is_some() { "    " } else { "  " };
                    let mut value = if let Some(item_idx) = ii {
                        self.get_item_field_value(*si, *item_idx, name)
                    } else {
                        self.get_scalar_field_value(*si, name)
                    };
                    if let Some(buf) = edit_buf
                        && i == self.row
                    {
                        value = buf.to_string();
                    }
                    let text = if value.is_empty() {
                        format!("{}{}", indent, name)
                    } else {
                        format!("{}{}: {}", indent, name, value)
                    };
                    items.push(RenderLine {
                        text,
                        selected: i == self.row,
                        is_section_hdr: false,
                    });
                }
                Row::EmptyHint => {
                    items.push(RenderLine {
                        text: "  (empty — press a to add)".to_string(),
                        selected: false,
                        is_section_hdr: false,
                    });
                }
            }
        }
        items
    }
}
