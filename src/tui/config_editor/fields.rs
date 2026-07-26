pub(super) const SECTION_NAMES: &[&str] = &[
    "agent",
    "tui",
    "session",
    "telegram",
    "providers",
    "models",
    "subagents",
    "schedule",
];

const AGENT_FIELDS: &[(&str, FieldKind)] = &[
    ("model", FieldKind::PickModels),
    ("tools", FieldKind::Comma),
    ("interfaces", FieldKind::Comma),
];

const TUI_FIELDS: &[(&str, FieldKind)] = &[
    ("tools_expanded", FieldKind::Bool),
    ("show_thinking", FieldKind::Bool),
];

const SESSION_FIELDS: &[(&str, FieldKind)] = &[("dir", FieldKind::Text)];

const TELEGRAM_FIELDS: &[(&str, FieldKind)] = &[("allowed_users", FieldKind::CommaI64)];

const PROVIDER_FIELDS: &[(&str, FieldKind)] =
    &[("id", FieldKind::Text), ("base_url", FieldKind::Text)];

const MODEL_FIELDS: &[(&str, FieldKind)] = &[
    ("id", FieldKind::Text),
    ("name", FieldKind::Text),
    ("provider", FieldKind::PickProviders),
    ("context_window", FieldKind::I32),
    ("max_output_tokens", FieldKind::I32),
    ("reasoning_effort", FieldKind::Text),
    ("input_price", FieldKind::F64),
    ("cached_input_price", FieldKind::F64),
    ("output_price", FieldKind::F64),
    ("prompt_cache", FieldKind::Bool),
    ("cache_ttl", FieldKind::Text),
];

const SUBAGENT_FIELDS: &[(&str, FieldKind)] = &[
    ("id", FieldKind::Text),
    ("description", FieldKind::Text),
    ("model", FieldKind::PickModels),
    ("tools", FieldKind::Comma),
];

const SCHEDULE_JOB_FIELDS: &[(&str, FieldKind)] = &[
    ("cron", FieldKind::Text),
    ("channel", FieldKind::Text),
    ("model", FieldKind::PickModels),
];

#[derive(Clone, Copy, PartialEq)]
pub(super) enum FieldKind {
    Text,
    I32,
    F64,
    Bool,
    Comma,
    CommaI64,
    PickModels,
    PickProviders,
}

impl super::ConfigScreen {
    pub(super) fn is_array_section(&self, idx: usize) -> bool {
        matches!(idx, 4..=7)
    }

    pub(super) fn array_len(&self, section_idx: usize) -> usize {
        match section_idx {
            4 => self.config.providers.len(),
            5 => self.config.models.len(),
            6 => self.config.subagents.len(),
            7 => self.config.schedule.jobs.len(),
            _ => 0,
        }
    }

    pub(super) fn field_count_for_section(&self, section_idx: usize) -> usize {
        match section_idx {
            0 => AGENT_FIELDS.len(),
            1 => TUI_FIELDS.len(),
            2 => SESSION_FIELDS.len(),
            3 => TELEGRAM_FIELDS.len(),
            4 => PROVIDER_FIELDS.len(),
            5 => MODEL_FIELDS.len(),
            6 => SUBAGENT_FIELDS.len(),
            7 => SCHEDULE_JOB_FIELDS.len(),
            _ => 0,
        }
    }

    pub(super) fn fields_for_section(&self, section_idx: usize) -> &[(&str, FieldKind)] {
        match section_idx {
            0 => AGENT_FIELDS,
            1 => TUI_FIELDS,
            2 => SESSION_FIELDS,
            3 => TELEGRAM_FIELDS,
            4 => PROVIDER_FIELDS,
            5 => MODEL_FIELDS,
            6 => SUBAGENT_FIELDS,
            7 => SCHEDULE_JOB_FIELDS,
            _ => &[],
        }
    }

    pub(super) fn get_agent_field(&self, name: &str) -> String {
        match name {
            "model" => self.config.agent.model.clone(),
            "tools" => self.config.agent.tools.join(", "),
            "interfaces" => self.config.agent.interfaces.join(", "),
            _ => String::new(),
        }
    }

    pub(super) fn set_agent_field(&mut self, name: &str, value: &str) -> Result<(), String> {
        match name {
            "model" => self.config.agent.model = value.to_string(),
            "tools" => self.config.agent.tools = parse_comma(value),
            "interfaces" => self.config.agent.interfaces = parse_comma(value),
            _ => {}
        }
        Ok(())
    }

    pub(super) fn get_tui_field(&self, name: &str) -> String {
        match name {
            "tools_expanded" => self.config.tui.tools_expanded.to_string(),
            "show_thinking" => self.config.tui.show_thinking.to_string(),
            _ => String::new(),
        }
    }

    pub(super) fn set_tui_field(&mut self, name: &str, value: &str) -> Result<(), String> {
        match name {
            "tools_expanded" => {
                self.config.tui.tools_expanded = value
                    .parse::<bool>()
                    .map_err(|_| format!("invalid bool: {}", value))?
            }
            "show_thinking" => {
                self.config.tui.show_thinking = value
                    .parse::<bool>()
                    .map_err(|_| format!("invalid bool: {}", value))?
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn get_session_field(&self, name: &str) -> String {
        match name {
            "dir" => self.config.session.dir.clone(),
            _ => String::new(),
        }
    }

    pub(super) fn set_session_field(&mut self, name: &str, value: &str) -> Result<(), String> {
        if name == "dir" {
            self.config.session.dir = value.to_string();
        }
        Ok(())
    }

    pub(super) fn get_telegram_field(&self, name: &str) -> String {
        match name {
            "allowed_users" => comma_i64(&self.config.telegram.allowed_users),
            _ => String::new(),
        }
    }

    pub(super) fn set_telegram_field(&mut self, name: &str, value: &str) -> Result<(), String> {
        if name == "allowed_users" {
            self.config.telegram.allowed_users =
                parse_comma_i64(value).map_err(|_| format!("invalid i64 list: {}", value))?
        }
        Ok(())
    }

    pub(super) fn get_provider_field(&self, idx: usize, name: &str) -> String {
        if let Some(p) = self.config.providers.get(idx) {
            match name {
                "id" => p.id.clone(),
                "base_url" => p.base_url.clone(),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    pub(super) fn set_provider_field(
        &mut self,
        idx: usize,
        name: &str,
        value: &str,
    ) -> Result<(), String> {
        if let Some(p) = self.config.providers.get_mut(idx) {
            match name {
                "id" => p.id = value.to_string(),
                "base_url" => p.base_url = value.to_string(),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn get_model_field(&self, idx: usize, name: &str) -> String {
        if let Some(m) = self.config.models.get(idx) {
            match name {
                "id" => m.id.clone(),
                "name" => m.name.clone(),
                "provider" => m.provider.clone(),
                "context_window" => m.context_window.to_string(),
                "max_output_tokens" => m.max_output_tokens.to_string(),
                "reasoning_effort" => m.reasoning_effort.clone(),
                "input_price" => format_f64(m.input_price),
                "cached_input_price" => format_f64(m.cached_input_price),
                "output_price" => format_f64(m.output_price),
                "prompt_cache" => m.prompt_cache.to_string(),
                "cache_ttl" => m.cache_ttl.clone(),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    pub(super) fn set_model_field(
        &mut self,
        idx: usize,
        name: &str,
        value: &str,
    ) -> Result<(), String> {
        if let Some(m) = self.config.models.get_mut(idx) {
            match name {
                "id" => m.id = value.to_string(),
                "name" => m.name = value.to_string(),
                "provider" => m.provider = value.to_string(),
                "context_window" => {
                    m.context_window = value
                        .parse::<i32>()
                        .map_err(|_| format!("invalid i32: {}", value))?
                }
                "max_output_tokens" => {
                    m.max_output_tokens = value
                        .parse::<i32>()
                        .map_err(|_| format!("invalid i32: {}", value))?
                }
                "reasoning_effort" => m.reasoning_effort = value.to_string(),
                "input_price" => {
                    m.input_price = value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid f64: {}", value))?
                }
                "cached_input_price" => {
                    m.cached_input_price = value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid f64: {}", value))?
                }
                "output_price" => {
                    m.output_price = value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid f64: {}", value))?
                }
                "prompt_cache" => {
                    m.prompt_cache = value
                        .parse::<bool>()
                        .map_err(|_| format!("invalid bool: {}", value))?
                }
                "cache_ttl" => m.cache_ttl = value.to_string(),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn get_subagent_field(&self, idx: usize, name: &str) -> String {
        if let Some(s) = self.config.subagents.get(idx) {
            match name {
                "id" => s.id.clone(),
                "description" => s.description.clone(),
                "model" => s.model.clone(),
                "tools" => s.tools.join(", "),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    pub(super) fn set_subagent_field(
        &mut self,
        idx: usize,
        name: &str,
        value: &str,
    ) -> Result<(), String> {
        if let Some(s) = self.config.subagents.get_mut(idx) {
            match name {
                "id" => s.id = value.to_string(),
                "description" => s.description = value.to_string(),
                "model" => s.model = value.to_string(),
                "tools" => s.tools = parse_comma(value),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn get_job_field(&self, idx: usize, name: &str) -> String {
        if let Some(j) = self.config.schedule.jobs.get(idx) {
            match name {
                "cron" => j.cron.clone(),
                "channel" => j.channel.clone(),
                "model" => j.model.clone(),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    pub(super) fn set_job_field(
        &mut self,
        idx: usize,
        name: &str,
        value: &str,
    ) -> Result<(), String> {
        if let Some(j) = self.config.schedule.jobs.get_mut(idx) {
            match name {
                "cron" => j.cron = value.to_string(),
                "channel" => j.channel = value.to_string(),
                "model" => j.model = value.to_string(),
                _ => {}
            }
        }
        Ok(())
    }

    pub(super) fn get_scalar_field_value(&self, section_idx: usize, field_name: &str) -> String {
        match section_idx {
            0 => self.get_agent_field(field_name),
            1 => self.get_tui_field(field_name),
            2 => self.get_session_field(field_name),
            3 => self.get_telegram_field(field_name),
            _ => String::new(),
        }
    }

    pub(super) fn set_scalar_field_value(
        &mut self,
        section_idx: usize,
        field_name: &str,
        value: &str,
    ) -> Result<(), String> {
        match section_idx {
            0 => self.set_agent_field(field_name, value),
            1 => self.set_tui_field(field_name, value),
            2 => self.set_session_field(field_name, value),
            3 => self.set_telegram_field(field_name, value),
            _ => Ok(()),
        }
    }

    pub(super) fn get_item_field_value(
        &self,
        section_idx: usize,
        item_idx: usize,
        field_name: &str,
    ) -> String {
        match section_idx {
            4 => self.get_provider_field(item_idx, field_name),
            5 => self.get_model_field(item_idx, field_name),
            6 => self.get_subagent_field(item_idx, field_name),
            7 => self.get_job_field(item_idx, field_name),
            _ => String::new(),
        }
    }

    pub(super) fn set_item_field_value(
        &mut self,
        section_idx: usize,
        item_idx: usize,
        field_name: &str,
        value: &str,
    ) -> Result<(), String> {
        match section_idx {
            4 => self.set_provider_field(item_idx, field_name, value),
            5 => self.set_model_field(item_idx, field_name, value),
            6 => self.set_subagent_field(item_idx, field_name, value),
            7 => self.set_job_field(item_idx, field_name, value),
            _ => Ok(()),
        }
    }

    pub(super) fn pick_options(&self, kind: FieldKind) -> Vec<String> {
        match kind {
            FieldKind::PickModels => self.config.models.iter().map(|m| m.id.clone()).collect(),
            FieldKind::PickProviders => {
                self.config.providers.iter().map(|p| p.id.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn cur_value_at(&self) -> String {
        if let Some((si, ii, _fi, name, _kind)) = self.field_at_cursor() {
            if let Some(item_idx) = ii {
                self.get_item_field_value(si, item_idx, &name)
            } else {
                self.get_scalar_field_value(si, &name)
            }
        } else {
            String::new()
        }
    }
}

pub(super) fn parse_comma(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

pub(super) fn parse_comma_i64(s: &str) -> Result<Vec<i64>, std::num::ParseIntError> {
    s.split(',')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<i64>())
        .collect()
}

pub(super) fn comma_i64(v: &[i64]) -> String {
    v.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn format_f64(v: f64) -> String {
    let s = format!("{:.6}", v);
    let trimmed = s.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{}0", trimmed)
    } else {
        trimmed.to_string()
    }
}

pub(super) fn unicode_display_width(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    UnicodeWidthStr::width(s)
}
