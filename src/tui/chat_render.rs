use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::chat_format::{TOOL_COLOR, format_tool_label, result_lang};
use crate::render::highlight;
use crate::tui::theme::COLORS;

const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

const RAINBOW: [Color; 6] = [
    Color::Rgb(0xFF, 0x6B, 0x6B),
    Color::Rgb(0xFF, 0xA9, 0x4D),
    Color::Rgb(0xFF, 0xD4, 0x3B),
    Color::Rgb(0x69, 0xDB, 0x7C),
    Color::Rgb(0x4D, 0xAB, 0xF7),
    Color::Rgb(0x97, 0x75, 0xFA),
];

pub fn thinking_indicator(
    ticks: usize,
    label: &str,
    elapsed: std::time::Duration,
) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, ch) in label.chars().enumerate() {
        let color = RAINBOW[(i + ticks) % RAINBOW.len()];
        spans.push(Span::styled(ch.to_string(), bold.fg(color)));
    }
    spans.push(Span::styled(
        format!(" ({})", fmt_duration(elapsed)),
        Style::default().fg(COLORS.placeholder),
    ));
    Line::from(spans)
}

fn fmt_duration(d: std::time::Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}h{}m", h, m)
    } else if m > 0 {
        format!("{}m{}s", m, s)
    } else {
        format!("{}s", s)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_tool_block(
    name: &str,
    args: &str,
    result: &str,
    error: &str,
    duration: &str,
    cwd: &str,
    subagent: &str,
    collapsed: bool,
    width: usize,
    indent: usize,
    ticks: usize,
) -> String {
    let mut out = String::new();
    let prefix = " ".repeat(indent);
    let field_prefix = " ".repeat(indent + 2);

    let label = format_tool_label(cwd, name, args);
    let label = if label.is_empty() {
        name.to_string()
    } else {
        label
    };
    let is_running = result.is_empty() && error.is_empty() && duration.is_empty();

    let symbol = if is_running {
        SPINNER[ticks % SPINNER.len()]
    } else if !error.is_empty() {
        "✗"
    } else {
        ""
    };

    let color = TOOL_COLOR;
    let (r, g, b) = crate::render::block::hex_to_rgb(color);
    let label_style = format!("\x1b[38;2;{r};{g};{b}m");
    let reset = "\x1b[0m";

    let mut name_str = format!("{}{}{}", label_style, label, reset);
    if !subagent.is_empty() {
        name_str = format!("[{}] {}", subagent, name_str);
    }

    out.push_str(&prefix);
    if !symbol.is_empty() {
        out.push_str(symbol);
        out.push(' ');
    }
    out.push_str(&name_str);

    if is_running {
        out.push_str(" running");
    } else if error.is_empty() && !duration.is_empty() {
        out.push(' ');
        out.push_str(duration);
    }

    if collapsed {
        return out;
    }

    if !result.is_empty() {
        let lang = result_lang(name, args);
        let rendered = if !lang.is_empty() {
            highlight::highlight(&lang, result)
        } else if let Ok(obj) = serde_json::from_str::<serde_json::Value>(result) {
            if let Ok(pretty) = serde_json::to_string_pretty(&obj) {
                highlight::highlight("json", &pretty)
            } else {
                result.to_string()
            }
        } else {
            result.to_string()
        };

        let lines: Vec<&str> = rendered.lines().collect();
        let max_lines = 50;
        let (content_lines, trunc_msg): (Vec<&str>, Option<String>) = if lines.len() > max_lines {
            (
                lines[..max_lines].to_vec(),
                Some(format!(
                    "... (truncated, {} more lines)",
                    lines.len() - max_lines
                )),
            )
        } else {
            (lines.to_vec(), None)
        };

        let (sr, sg, sb) = crate::render::block::hex_to_rgb(crate::render::theme::VESPER.surface);
        let (pr, pg, pb) =
            crate::render::block::hex_to_rgb(crate::render::theme::VESPER.placeholder);
        let bg = format!("\x1b[48;2;{sr};{sg};{sb}m");
        let dim = format!("\x1b[38;2;{pr};{pg};{pb}m");
        let reset = "\x1b[0m";
        let panel_w = width.saturating_sub(indent);
        let lead = " ".repeat(indent);
        let pad_line = format!("{lead}{bg}{}{reset}", " ".repeat(panel_w));

        out.push('\n');
        out.push_str(&pad_line);
        for line in &content_lines {
            let line = crate::render::block::truncate(line, panel_w.saturating_sub(2), "…");
            let line = line.replace(reset, &format!("{reset}{bg}"));
            let vw = crate::render::block::visible_width(&line);
            let pad = panel_w.saturating_sub(vw + 1);
            out.push('\n');
            out.push_str(&lead);
            out.push_str(&format!("{bg} {line}{}{reset}", " ".repeat(pad)));
        }
        if let Some(msg) = trunc_msg {
            let vw = crate::render::block::visible_width(&msg);
            let pad = panel_w.saturating_sub(vw + 1);
            out.push('\n');
            out.push_str(&lead);
            out.push_str(&format!("{bg} {dim}{msg}{}{reset}", " ".repeat(pad)));
        }
        out.push('\n');
        out.push_str(&pad_line);
        out = out.trim_end().to_string();
    }

    if !error.is_empty() {
        out.push('\n');
        out.push_str(&field_prefix);
        out.push_str(error);
    }

    out
}

pub fn git_branch(cwd: &str) -> Option<String> {
    let mut dir: &std::path::Path = std::path::Path::new(cwd);
    loop {
        let head = dir.join(".git").join("HEAD");
        if let Ok(content) = std::fs::read_to_string(&head) {
            let trimmed = content.trim();
            if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                return Some(branch.to_string());
            }
            if trimmed.len() >= 7 {
                return Some(trimmed[..7].to_string());
            }
            return Some(trimmed.to_string());
        }
        dir = dir.parent()?;
    }
}

pub fn render_top_bar(f: &mut Frame, area: Rect, cwd: &str, model: &str) {
    let gray = COLORS.placeholder;
    let bright = COLORS.muted;

    let mut spans = Vec::new();
    if let Some(branch) = git_branch(cwd) {
        spans.push(Span::styled(
            format!(" {branch}"),
            Style::default().fg(bright),
        ));
    }
    let display_cwd = crate::tui::chat::shorten_cwd_string(cwd);
    spans.push(Span::styled(
        format!("  {display_cwd}"),
        Style::default().fg(gray),
    ));

    if !model.is_empty() {
        let left_w: usize = spans.iter().map(|s| s.width()).sum();
        let model_text = format!(" {model}");
        let model_w = model_text.len();
        let pad = (area.width as usize)
            .saturating_sub(left_w)
            .saturating_sub(model_w);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(model_text, Style::default().fg(gray)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn render_shortcuts_bar(f: &mut Frame, area: Rect, hints: &[(&str, &str)]) {
    let key_style = Style::default()
        .fg(COLORS.muted)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(COLORS.placeholder);
    let sep = Span::styled("  │  ", Style::default().fg(COLORS.placeholder));

    let mut spans = Vec::new();
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(sep.clone());
        }
        spans.push(Span::styled(format!(" {key} "), key_style));
        spans.push(Span::styled(*label, label_style));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::block::strip_ansi;

    #[test]
    fn test_thinking_indicator_rainbow_colors_per_char() {
        let line = thinking_indicator(0, "ab", std::time::Duration::from_secs(5));
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content.as_ref(), "a");
        assert_eq!(line.spans[0].style.fg, Some(RAINBOW[0]));
        assert_eq!(line.spans[1].content.as_ref(), "b");
        assert_eq!(line.spans[1].style.fg, Some(RAINBOW[1]));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_thinking_indicator_empty_label() {
        let line = thinking_indicator(0, "", std::time::Duration::from_secs(5));
        assert_eq!(line.spans.len(), 1);
        assert!(line.spans[0].content.contains("(5s)"));
    }

    #[test]
    fn test_thinking_indicator_offset_shifts_colors() {
        let base = thinking_indicator(0, "ab", std::time::Duration::from_secs(5));
        let shifted = thinking_indicator(1, "ab", std::time::Duration::from_secs(5));
        assert_eq!(base.spans[0].style.fg, Some(RAINBOW[0]));
        assert_eq!(shifted.spans[0].style.fg, Some(RAINBOW[1]));
        assert_eq!(base.spans[1].style.fg, shifted.spans[0].style.fg);
    }

    #[test]
    fn test_tool_result_panel_wraps_in_background() {
        let out = render_tool_block(
            "bash",
            "{\"command\":\"ls\"}",
            "file_a.rs\nfile_b.rs",
            "",
            "5s",
            ".",
            "",
            false,
            40,
            0,
            0,
        );
        let surface_bg = {
            let (r, g, b) = crate::render::block::hex_to_rgb(crate::render::theme::VESPER.surface);
            format!("\x1b[48;2;{r};{g};{b}m")
        };
        let plain = strip_ansi(&out);
        let lines: Vec<&str> = plain.lines().collect();
        assert!(
            lines.len() >= 5,
            "expected label + 2 pads + 2 result lines, got {}",
            lines.len()
        );
        assert_eq!(lines[0], "ls 5s");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[1].len(), 40);
        assert!(lines[2].starts_with(" file_a.rs"));
        assert_eq!(lines[2].len(), 40);
        assert!(lines[3].starts_with(" file_b.rs"));
        assert_eq!(lines[3].len(), 40);
        assert_eq!(lines[4].trim(), "");
        assert_eq!(lines[4].len(), 40);
        assert!(
            out.contains(&surface_bg),
            "result block must use surface background"
        );
    }

    #[test]
    fn test_tool_result_panel_respects_indent() {
        let out = render_tool_block("read_file", "", "hello", "", "1s", ".", "", false, 40, 2, 0);
        let plain = strip_ansi(&out);
        let lines: Vec<&str> = plain.lines().collect();
        assert!(lines[1].starts_with("  "), "pad line must be indented by 2");
        assert!(
            lines[2].starts_with("   hello"),
            "content must be indented by 3"
        );
    }

    #[test]
    fn test_tool_result_panel_truncation_message_present() {
        let big = (0..60)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = render_tool_block("bash", "", &big, "", "1s", ".", "", false, 80, 0, 0);
        assert!(out.contains("... (truncated, 10 more lines)"));
    }

    #[test]
    fn test_tool_result_collapsed_has_no_panel() {
        let out = render_tool_block("bash", "", "some output", "", "1s", ".", "", true, 80, 0, 0);
        let surface_bg = {
            let (r, g, b) = crate::render::block::hex_to_rgb(crate::render::theme::VESPER.surface);
            format!("\x1b[48;2;{r};{g};{b}m")
        };
        assert!(
            !out.contains(&surface_bg),
            "collapsed block must not render panel"
        );
    }
}
