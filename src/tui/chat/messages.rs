use super::textarea::textarea_cursor_xy;
use super::{ChatScreen, fit_height, fmt_cost, shorten_cwd_string};
use crate::message::Message;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Wrap},
};

use super::super::chat_dropdowns::render_dropdown;
use super::super::chat_handlers::{ChatMsg, Segment};
use super::super::chat_render::{render_tool_block, thinking_indicator};
use super::super::theme::COLORS;

impl ChatScreen {
    pub fn load_messages(
        &mut self,
        msgs: &[Message],
        children: &std::collections::HashMap<String, Vec<Message>>,
    ) {
        let mut pending_tools: Vec<(String, String, String)> = Vec::new();

        for msg in msgs {
            match msg.role {
                crate::message::Role::User => {
                    self.messages.push(ChatMsg::user(&msg.content));
                    self.history.push(msg.content.clone());
                }
                crate::message::Role::Assistant => {
                    let last_was_assistant = self
                        .messages
                        .last()
                        .map(|m| m.role == "assistant")
                        .unwrap_or(false);

                    if last_was_assistant {
                        let last = self.messages.last_mut().unwrap();
                        if !msg.reasoning_content.is_empty() {
                            last.segments
                                .push(Segment::thinking(&msg.reasoning_content));
                        }
                        if !msg.content.is_empty() {
                            last.segments.push(Segment::prose(&msg.content));
                        }
                        for tc in &msg.tool_calls {
                            pending_tools.push((
                                tc.id.clone(),
                                tc.function.name.clone(),
                                tc.function.arguments.clone(),
                            ));
                        }
                    } else {
                        let mut segments = Vec::new();
                        if !msg.reasoning_content.is_empty() {
                            segments.push(Segment::thinking(&msg.reasoning_content));
                        }
                        if !msg.content.is_empty() {
                            segments.push(Segment::prose(&msg.content));
                        }
                        for tc in &msg.tool_calls {
                            pending_tools.push((
                                tc.id.clone(),
                                tc.function.name.clone(),
                                tc.function.arguments.clone(),
                            ));
                        }
                        self.messages.push(ChatMsg::assistant(segments));
                    }
                }
                crate::message::Role::Tool => {
                    if let Some((id, name, args)) = pending_tools.first() {
                        let id = id.clone();
                        let name = name.clone();
                        let args = args.clone();
                        pending_tools.remove(0);

                        let last = self.messages.last_mut().unwrap();
                        if last.role == "assistant" {
                            last.segments.push(Segment::tool(
                                &name,
                                &args,
                                &msg.content,
                                "",
                                &msg.tool_duration,
                                &self.cwd,
                                "",
                            ));

                            if name == "delegate"
                                && !id.is_empty()
                                && let Some(child_msgs) = children.get(&id)
                            {
                                let delegate_seg = last.segments.last_mut().unwrap();
                                for child_msg in child_msgs {
                                    if child_msg.role == crate::message::Role::Tool {
                                        delegate_seg.children.push(Segment::tool(
                                            &child_msg.name,
                                            "",
                                            &child_msg.content,
                                            "",
                                            &child_msg.tool_duration,
                                            &self.cwd,
                                            "",
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        self.render_messages();
    }

    pub(super) fn render_messages_area(&mut self, f: &mut Frame, msg_area: Rect) {
        let mut all_lines: Vec<Line> = Vec::new();
        all_lines.push(Line::from(""));

        for msg in &self.messages {
            let rendered = if msg.rendered.is_empty() {
                self.render_message_ansi(msg)
            } else {
                msg.rendered.clone()
            };

            let text = crate::render::block::ansi_to_text(&rendered);

            if msg.role == "user" {
                all_lines.push(Line::from(""));
            }
            for line in text.lines {
                all_lines.push(line);
            }
            if msg.role == "user" {
                all_lines.push(Line::from(""));
            }
        }

        if !self.live_blocks.is_empty() {
            let live = self.render_live_turn();
            let t = crate::render::block::ansi_to_text(&live);
            for line in t.lines {
                all_lines.push(line);
            }
        }

        all_lines.push(Line::from(""));

        let total_lines = all_lines.len();
        let visible = msg_area.height as usize;
        let max_offset = total_lines.saturating_sub(visible);

        if self.scroll_to_bottom {
            self.viewport_offset = max_offset;
            self.scroll_to_bottom = false;
            self.user_scrolled_up = false;
        } else if !self.user_scrolled_up {
            self.viewport_offset = max_offset;
        }
        if self.viewport_offset > max_offset {
            self.viewport_offset = max_offset;
            self.user_scrolled_up = false;
        }

        let text = Text::from(all_lines);
        let paragraph = Paragraph::new(text).scroll((self.viewport_offset as u16, 0));
        f.render_widget(paragraph, msg_area);
    }

    fn render_message_ansi(&self, msg: &ChatMsg) -> String {
        match msg.role.as_str() {
            "user" => {
                let full_w = (self.width as usize).saturating_sub(4);
                let (r, g, b) =
                    crate::render::block::hex_to_rgb(crate::render::theme::VESPER.surface);
                let bg = format!("\x1b[48;2;{r};{g};{b}m");
                let reset = "\x1b[0m";
                let pad_line = format!("{bg}{}{reset}", " ".repeat(full_w));
                let rendered = self.renderer.render(msg.content.trim());
                let re_bg = rendered.replace(reset, &format!("{reset}{bg}"));
                let mut lines = vec![pad_line.clone()];
                for line in re_bg.lines() {
                    let vw = crate::render::block::visible_width(line);
                    let pad = full_w.saturating_sub(vw + 1);
                    lines.push(format!("{bg} {line}{}{reset}", " ".repeat(pad)));
                }
                lines.push(pad_line);
                lines.join("\n")
            }
            "assistant" => self.render_assistant_turn(msg),
            "error" => {
                let full_w = (self.width as usize).saturating_sub(4);
                let (r, g, b) = crate::render::block::hex_to_rgb("#3a1d1d");
                let bg = format!("\x1b[48;2;{r};{g};{b}m");
                let red = "\x1b[38;2;247;118;142m";
                let reset = "\x1b[0m";
                let pad_line = format!("{bg}{}{reset}", " ".repeat(full_w));
                let mut lines = vec![pad_line.clone()];
                for raw in msg.content.trim().split('\n') {
                    for line in
                        crate::render::block::wordwrap(raw, full_w.saturating_sub(2), "").lines()
                    {
                        let vw = crate::render::block::visible_width(line);
                        let pad = full_w.saturating_sub(vw + 1);
                        lines.push(format!("{bg} {red}{line}{}{reset}", " ".repeat(pad)));
                    }
                }
                lines.push(pad_line);
                lines.join("\n")
            }
            _ => String::new(),
        }
    }

    fn render_assistant_turn(&self, msg: &ChatMsg) -> String {
        let mut parts = Vec::new();
        let diamond = "\x1b[38;2;187;154;247m◆\x1b[0m ";
        let bar = "\x1b[38;2;187;154;247m";
        let reset = "\x1b[0m";

        for seg in &msg.segments {
            let content = match seg.kind.as_str() {
                "thinking" => {
                    if self.show_thinking {
                        let rendered = self.renderer.render(&seg.content);
                        let mut out = String::new();
                        out.push_str(&format!("{bar}┃{reset} {diamond}Thinking…\n"));
                        out.push_str(&format!("{bar}┃{reset} "));
                        out.push_str(
                            &rendered
                                .lines()
                                .collect::<Vec<_>>()
                                .join(&format!("\n{bar}┃{reset} ")),
                        );
                        out
                    } else {
                        continue;
                    }
                }
                "prose" => self.renderer.render(&seg.content),
                "tool" => {
                    let collapsed = !self.tools_expanded;
                    let width = (self.width as usize).saturating_sub(4);
                    let mut out = render_tool_block(
                        &seg.tool_name,
                        &seg.tool_args,
                        &seg.tool_result,
                        &seg.tool_error,
                        &seg.tool_duration,
                        &self.cwd,
                        &seg.tool_subagent,
                        collapsed,
                        width,
                        0,
                        0,
                    );
                    if self.show_subagent_calls && !seg.children.is_empty() {
                        if collapsed {
                            let names: Vec<&str> =
                                seg.children.iter().map(|c| c.tool_name.as_str()).collect();
                            out.push_str(&tool_children_summary(&names));
                        } else {
                            for child in &seg.children {
                                out.push('\n');
                                out.push_str(&render_tool_block(
                                    &child.tool_name,
                                    &child.tool_args,
                                    &child.tool_result,
                                    &child.tool_error,
                                    &child.tool_duration,
                                    &self.cwd,
                                    "",
                                    false,
                                    width,
                                    2,
                                    0,
                                ));
                            }
                        }
                    }
                    out
                }
                _ => continue,
            };
            if !content.is_empty() {
                parts.push(content);
            }
        }
        let mut rendered = parts.join("\n\n");
        if msg.stopped {
            rendered.push_str("\n\x1b[2m[stopped]\x1b[0m");
        }
        rendered
    }

    fn render_live_turn(&self) -> String {
        let mut parts = Vec::new();
        let diamond = "\x1b[38;2;187;154;247m◆\x1b[0m ";
        let bar = "\x1b[38;2;187;154;247m";
        let reset = "\x1b[0m";

        for lb in &self.live_blocks {
            let content = match lb.kind.as_str() {
                "prose" => self.renderer.render(&lb.raw),
                "thinking" => {
                    if !self.show_thinking {
                        continue;
                    }
                    let rendered = self.renderer.render(&lb.raw);
                    let mut out = String::new();
                    out.push_str(&format!("{bar}┃{reset} {diamond}Thinking…\n"));
                    out.push_str(&format!("{bar}┃{reset} "));
                    out.push_str(
                        &rendered
                            .lines()
                            .collect::<Vec<_>>()
                            .join(&format!("\n{bar}┃{reset} ")),
                    );
                    out
                }
                "tool" => {
                    let collapsed = !self.tools_expanded;
                    let width = (self.width as usize).saturating_sub(4);
                    let mut out = render_tool_block(
                        &lb.tool_name,
                        &lb.tool_args,
                        &lb.tool_result,
                        &lb.tool_error,
                        &lb.tool_duration,
                        &self.cwd,
                        &lb.tool_subagent,
                        collapsed,
                        width,
                        0,
                        self.wait_ticks,
                    );
                    if self.show_subagent_calls && !lb.children.is_empty() {
                        if collapsed {
                            let names: Vec<&str> =
                                lb.children.iter().map(|c| c.tool_name.as_str()).collect();
                            out.push_str(&tool_children_summary(&names));
                        } else {
                            for child in &lb.children {
                                out.push('\n');
                                out.push_str(&render_tool_block(
                                    &child.tool_name,
                                    &child.tool_args,
                                    &child.tool_result,
                                    &child.tool_error,
                                    &child.tool_duration,
                                    &self.cwd,
                                    "",
                                    false,
                                    width,
                                    2,
                                    self.wait_ticks,
                                ));
                            }
                        }
                    }
                    out
                }
                _ => continue,
            };
            if !content.is_empty() {
                parts.push(content);
            }
        }
        parts.join("\n\n")
    }

    pub fn rerender_all(&mut self) {
        for msg in &mut self.messages {
            msg.rendered.clear();
        }
        self.render_messages();
    }

    pub fn render_messages(&mut self) {
        let mut rendered_msgs: Vec<String> = Vec::with_capacity(self.messages.len());
        for msg in &self.messages {
            let r = if msg.rendered.is_empty() {
                self.render_message_ansi(msg)
            } else {
                msg.rendered.clone()
            };
            rendered_msgs.push(r);
        }
        for (msg, r) in self.messages.iter_mut().zip(rendered_msgs) {
            if msg.rendered.is_empty() {
                msg.rendered = r;
            }
        }
    }

    pub(super) fn bottom_bar_height(&self) -> u16 {
        if self.waiting {
            let mut h = 1u16;
            if self.active_modal == super::Modal::Command {
                h += (self.command_dropdown.items.len() + 3) as u16;
            }
            if self.active_modal == super::Modal::Model {
                h += (self.model_dropdown.items.len() + 3) as u16;
            }
            if self.active_modal == super::Modal::Tree {
                h += 6;
            }
            return h;
        }
        let mut h = self.textarea_height + 2;
        if self.active_modal == super::Modal::Template {
            h += 8;
        }
        if self.active_modal == super::Modal::File {
            h += 11;
        }
        if self.active_modal == super::Modal::Command {
            h += (self.command_dropdown.items.len() + 3) as u16;
        }
        if self.active_modal == super::Modal::Model {
            h += (self.model_dropdown.items.len() + 3) as u16;
        }
        if self.active_modal == super::Modal::Tree {
            h += 6;
        }
        h
    }

    pub(super) fn render_bottom_bar(&self, f: &mut Frame, area: Rect) {
        let bottom_h = self.bottom_bar_height().min(area.height);
        let bottom_y = area.height.saturating_sub(bottom_h);
        let bottom_area = Rect::new(area.x, bottom_y, area.width, bottom_h);

        let mut y_offset = 0u16;

        if self.active_modal == super::Modal::Template {
            let h = 8u16;
            let top = bottom_area.y + y_offset;
            if let Some(h2) = fit_height(area, top, h) {
                let modal_area = Rect::new(
                    bottom_area.x + 1,
                    top,
                    bottom_area.width.saturating_sub(2),
                    h2,
                );
                render_dropdown(
                    f,
                    modal_area,
                    &self.template_dropdown,
                    "Templates",
                    " No matches",
                    |item, _| {
                        let name = format!("/{}", item.template.name);
                        let desc = if item.template.description.is_empty() {
                            String::new()
                        } else {
                            format!("  {}", item.template.description)
                        };
                        Line::styled(
                            format!(" {}{}", name, desc),
                            Style::default().fg(COLORS.muted),
                        )
                    },
                    true,
                );
            }
            y_offset += h;
        }
        if self.active_modal == super::Modal::File {
            let h = 11u16;
            let top = bottom_area.y + y_offset;
            if let Some(h2) = fit_height(area, top, h) {
                let modal_area = Rect::new(
                    bottom_area.x + 1,
                    top,
                    bottom_area.width.saturating_sub(2),
                    h2,
                );
                render_dropdown(
                    f,
                    modal_area,
                    &self.file_dropdown,
                    "Files",
                    " No matches",
                    |item, _| {
                        Line::styled(
                            format!(" {}", item.label),
                            Style::default().fg(COLORS.muted),
                        )
                    },
                    true,
                );
            }
            y_offset += h;
        }
        if self.active_modal == super::Modal::Command {
            let h = (self.command_dropdown.items.len() + 3) as u16;
            let top = bottom_area.y + y_offset;
            if let Some(h2) = fit_height(area, top, h) {
                let modal_area = Rect::new(
                    bottom_area.x + 1,
                    top,
                    bottom_area.width.saturating_sub(2),
                    h2,
                );
                let title = if self.command_query.is_empty() {
                    "Commands".to_string()
                } else {
                    format!("Commands: {}", self.command_query)
                };
                render_dropdown(
                    f,
                    modal_area,
                    &self.command_dropdown,
                    &title,
                    " No matches",
                    |item, _| {
                        Line::styled(
                            format!(" {}", item.label),
                            Style::default().fg(COLORS.muted),
                        )
                    },
                    false,
                );
            }
            y_offset += h;
        }

        if self.active_modal == super::Modal::Model {
            let h = (self.model_dropdown.items.len() + 3) as u16;
            let top = bottom_area.y + y_offset;
            if let Some(h2) = fit_height(area, top, h) {
                let modal_area = Rect::new(
                    bottom_area.x + 1,
                    top,
                    bottom_area.width.saturating_sub(2),
                    h2,
                );
                render_dropdown(
                    f,
                    modal_area,
                    &self.model_dropdown,
                    "Switch Model",
                    " No models configured",
                    |item, _| {
                        let marker = if item.label == self.model_name {
                            "● "
                        } else {
                            "  "
                        };
                        Line::styled(
                            format!("{}{}", marker, item.label),
                            Style::default().fg(COLORS.muted),
                        )
                    },
                    true,
                );
            }
            y_offset += h;
        }

        if self.waiting {
            let mut indicator =
                thinking_indicator(self.wait_ticks, "Thinking…", self.wait_start.elapsed());
            indicator.push_span(Span::styled(
                "  Esc to stop",
                Style::default().fg(COLORS.placeholder),
            ));
            let top = bottom_area.y + y_offset;
            if fit_height(area, top, 1).is_some() {
                let thinking_area = Rect::new(
                    bottom_area.x + 2,
                    top,
                    bottom_area.width.saturating_sub(3),
                    1,
                );
                f.render_widget(
                    Paragraph::new(indicator).alignment(Alignment::Left),
                    thinking_area,
                );
            }
        } else {
            let prompt_h = self.textarea_height + 2;
            let top = bottom_area.y + y_offset;
            let input = if self.textarea.is_empty() {
                "Send a message…"
            } else {
                &self.textarea
            };
            let input_color = if self.textarea.is_empty() {
                COLORS.placeholder
            } else {
                COLORS.fg
            };

            let branch_or_cwd = crate::tui::chat_render::git_branch(&self.cwd)
                .unwrap_or_else(|| shorten_cwd_string(&self.cwd));
            let border_color = COLORS.border;
            let dim = Style::default().fg(COLORS.placeholder);

            let ctx_frac = if self.context_window > 0 {
                self.total_tokens as f64 / self.context_window as f64
            } else {
                0.0
            };
            let cache_frac = if self.total_tokens > 0 {
                self.cache_hit_tokens as f64 / self.total_tokens as f64
            } else {
                0.0
            };
            let ctx_color = if ctx_frac >= 0.9 {
                COLORS.error
            } else {
                COLORS.green
            };
            let mut stats_spans: Vec<Span<'static>> = Vec::new();
            stats_spans.push(Span::styled(" ", dim));
            stats_spans.extend(bar_with_text(
                ctx_frac,
                &format!("{:.0}%", ctx_frac * 100.0),
                ctx_color,
            ));
            if self.cache_hit_tokens > 0 {
                stats_spans.push(Span::styled(" · ", dim));
                stats_spans.extend(bar_with_text(
                    cache_frac,
                    &format!("{:.0}%", cache_frac * 100.0),
                    COLORS.green,
                ));
            }
            if self.total_cost > 0.0 {
                stats_spans.push(Span::styled(
                    format!(" · {} ", fmt_cost(self.total_cost)),
                    dim,
                ));
            } else {
                stats_spans.push(Span::styled(" ", dim));
            }
            let stats = Line::from(stats_spans).right_aligned();
            let info = Line::styled(format!(" {} · {} ", self.model_name, branch_or_cwd), dim)
                .right_aligned();

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color))
                .padding(Padding::new(1, 1, 0, 0))
                .title_top(stats)
                .title_bottom(info);

            if let Some(h2) = fit_height(area, top, prompt_h) {
                let prompt_area = Rect::new(
                    bottom_area.x + 1,
                    top,
                    bottom_area.width.saturating_sub(2),
                    h2,
                );
                let content_area = block.inner(prompt_area);
                f.render_widget(block, prompt_area);
                if content_area.height > 0 && content_area.width > 0 {
                    f.render_widget(
                        Paragraph::new(input.to_string())
                            .style(Style::default().fg(input_color))
                            .wrap(Wrap { trim: true })
                            .scroll((self.textarea_scroll as u16, 0)),
                        content_area,
                    );
                    if self.blink_on {
                        let (crow, ccol) =
                            textarea_cursor_xy(&self.textarea, self.cursor, content_area.width);
                        let display_row = crow.saturating_sub(self.textarea_scroll as u16);
                        let cx = content_area.x + ccol.min(content_area.width);
                        let cy =
                            content_area.y + display_row.min(content_area.height.saturating_sub(1));
                        f.set_cursor_position((cx, cy));
                    }
                }
            }
        }
    }
}

fn bar_with_text(fraction: f64, label: &str, full_color: Color) -> Vec<Span<'static>> {
    const WIDTH: usize = 10;
    let filled = ((fraction.clamp(0.0, 1.0) * WIDTH as f64).round() as usize).min(WIDTH);
    let track = COLORS.selected;
    let chars: Vec<char> = label.chars().collect();
    let label_len = chars.len().min(WIDTH);
    let label_start = WIDTH.saturating_sub(label_len).div_ceil(2);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut i = 0;
    while i < WIDTH {
        let is_filled = i < filled;
        let (bg, fg) = if is_filled {
            (full_color, COLORS.bg)
        } else {
            (track, COLORS.muted)
        };
        let mut run = String::new();
        while i < WIDTH && (i < filled) == is_filled {
            let label_pos = i as isize - label_start as isize;
            let c = if label_pos >= 0 && (label_pos as usize) < label_len {
                chars[label_pos as usize]
            } else {
                ' '
            };
            run.push(c);
            i += 1;
        }
        spans.push(Span::styled(run, Style::default().fg(fg).bg(bg)));
    }
    spans
}

fn tool_children_summary(names: &[&str]) -> String {
    let n = names.len();
    let unit = if n == 1 { "call" } else { "calls" };
    let mut seen: Vec<&str> = Vec::new();
    for nm in names {
        if !seen.contains(nm) {
            seen.push(nm);
        }
    }
    format!(" ({} {}: {})", n, unit, seen.join(", "))
}

#[cfg(test)]
mod summary_tests {
    use super::tool_children_summary;

    #[test]
    fn summary_lists_distinct_tool_names_with_count() {
        let s = tool_children_summary(&["bash", "read_file", "bash"]);
        assert_eq!(s, " (3 calls: bash, read_file)");
    }

    #[test]
    fn summary_single_call() {
        let s = tool_children_summary(&["grep"]);
        assert_eq!(s, " (1 call: grep)");
    }

    #[test]
    fn summary_empty() {
        let s: &[&str] = &[];
        assert_eq!(tool_children_summary(s), " (0 calls: )");
    }
}
