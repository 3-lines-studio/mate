use crate::prompts::Template;
use crate::tui::theme::COLORS;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
};

fn fold_byte(b: u8) -> u8 {
    if b.is_ascii_uppercase() { b + 32 } else { b }
}

fn is_alnum_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

pub fn fuzzy_score(query: &str, hay: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    if query.is_ascii() && hay.is_ascii() {
        return fuzzy_score_ascii(query.as_bytes(), hay.as_bytes());
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let h: Vec<char> = hay.to_lowercase().chars().collect();
    if q.len() > h.len() {
        return None;
    }
    let mut qi = 0usize;
    let mut score: i64 = 0;
    let mut prev_matched = false;
    let mut first: Option<usize> = None;
    for (hi, hc) in h.iter().enumerate() {
        if qi < q.len() && hc == &q[qi] {
            first.get_or_insert(hi);
            score += if prev_matched { 16 } else { 1 };
            if hi == 0 || !h[hi - 1].is_alphanumeric() {
                score += 8;
            }
            prev_matched = true;
            qi += 1;
        } else {
            prev_matched = false;
        }
    }
    if qi != q.len() {
        return None;
    }
    if let Some(f) = first {
        score -= (f as i64).min(20);
    }
    Some(score)
}

fn fuzzy_score_ascii(q: &[u8], h: &[u8]) -> Option<i64> {
    if q.len() > h.len() {
        return None;
    }
    let mut qi = 0usize;
    let mut score: i64 = 0;
    let mut prev_matched = false;
    let mut first: Option<usize> = None;
    for (hi, &hb) in h.iter().enumerate() {
        let hc = fold_byte(hb);
        if qi < q.len() && hc == fold_byte(q[qi]) {
            first.get_or_insert(hi);
            score += if prev_matched { 16 } else { 1 };
            if hi == 0 || !is_alnum_byte(h[hi - 1]) {
                score += 8;
            }
            prev_matched = true;
            qi += 1;
        } else {
            prev_matched = false;
        }
    }
    if qi != q.len() {
        return None;
    }
    if let Some(f) = first {
        score -= (f as i64).min(20);
    }
    Some(score)
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("New Session", "new"),
    ("Back to Sessions", "back"),
    ("Turn Tree", "tree"),
    ("Toggle Tool Results", "tools"),
    ("Toggle Subagent Calls", "subagents"),
    ("Toggle Thinking", "thinking"),
    ("Switch Model", "model"),
    ("Copy Last Response", "copy-last"),
    ("Export as Markdown", "export-md"),
    ("Edit Config", "config"),
    ("Quit", "quit"),
];

#[derive(Clone)]
pub struct LabeledItem {
    pub label: String,
    pub value: String,
}

#[derive(Clone)]
pub struct TemplateItem {
    pub template: Template,
    pub search_key: String,
}

#[derive(Clone)]
pub struct TreeItem {
    pub turn_id: String,
    pub label: String,
    pub depth: usize,
    pub is_last: bool,
    pub ancestors: Vec<bool>,
    pub is_current: bool,
}

#[derive(Clone)]
pub struct Dropdown<T: Clone> {
    pub items: Vec<T>,
    pub selected: usize,
    pub visible: bool,
}

impl<T: Clone> Dropdown<T> {
    pub fn new() -> Self {
        Dropdown {
            items: Vec::new(),
            selected: 0,
            visible: false,
        }
    }

    pub fn up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.items.get(self.selected)
    }
}

impl<T: Clone> Default for Dropdown<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn render_dropdown<T: Clone>(
    f: &mut Frame,
    area: Rect,
    dropdown: &Dropdown<T>,
    title: &str,
    empty_text: &str,
    fmt: impl Fn(&T, bool) -> Line<'_>,
    use_state: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLORS.border))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if dropdown.items.is_empty() && !empty_text.is_empty() {
        f.render_widget(
            Paragraph::new(empty_text)
                .style(Style::default().fg(COLORS.placeholder))
                .alignment(Alignment::Left),
            inner,
        );
        return;
    }

    let items: Vec<ListItem> = dropdown
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == dropdown.selected;
            let line = fmt(item, is_sel);
            if is_sel {
                ListItem::new(line).style(Style::default().bg(COLORS.selected).fg(COLORS.accent))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    if use_state {
        let mut state = ListState::default();
        state.select(Some(dropdown.selected));
        f.render_stateful_widget(List::new(items), inner, &mut state);
    } else {
        f.render_widget(List::new(items), inner);
    }
}
