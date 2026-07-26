use super::ChatScreen;

use super::super::chat_dropdowns::{COMMANDS, LabeledItem, TemplateItem, TreeItem, fuzzy_score};

impl ChatScreen {
    pub fn open_command_dropdown(&mut self) {
        self.active_modal = super::Modal::Command;
        self.command_query.clear();
        self.command_dropdown.selected = 0;
        self.filter_command_dropdown();
    }

    pub fn filter_command_dropdown(&mut self) {
        let q = self.command_query.as_str();
        if q.is_empty() {
            self.command_dropdown.items = COMMANDS
                .iter()
                .map(|(l, a)| LabeledItem {
                    label: l.to_string(),
                    value: a.to_string(),
                })
                .collect();
        } else {
            let mut scored: Vec<(i64, LabeledItem)> = COMMANDS
                .iter()
                .filter_map(|(l, a)| {
                    fuzzy_score(q, l).map(|s| {
                        (
                            s,
                            LabeledItem {
                                label: l.to_string(),
                                value: a.to_string(),
                            },
                        )
                    })
                })
                .collect();
            scored.sort_by_key(|b| std::cmp::Reverse(b.0));
            self.command_dropdown.items = scored.into_iter().map(|(_, t)| t).collect();
        }
        if self.command_dropdown.selected >= self.command_dropdown.items.len() {
            self.command_dropdown.selected = self.command_dropdown.items.len().saturating_sub(1);
        }
        self.command_dropdown.visible = true;
    }

    pub fn open_model_dropdown(&mut self, models: &[crate::config::ModelConfig]) {
        self.active_modal = super::Modal::Model;
        self.model_dropdown.items = models
            .iter()
            .map(|m| LabeledItem {
                label: m.name.clone(),
                value: m.id.clone(),
            })
            .collect();
        self.model_dropdown.selected = self
            .model_dropdown
            .items
            .iter()
            .position(|n| n.label == self.model_name)
            .unwrap_or(0);
        self.model_dropdown.visible = true;
    }

    pub fn close_dropdowns(&mut self) {
        self.active_modal = super::Modal::None;
        self.command_dropdown.visible = false;
        self.model_dropdown.visible = false;
        self.template_dropdown.visible = false;
        self.file_dropdown.visible = false;
        self.tree_dropdown.visible = false;
    }

    pub fn open_template_dropdown(&mut self, query: &str) {
        self.active_modal = super::Modal::Template;
        self.filter_template_dropdown(query);
    }

    pub fn filter_template_dropdown(&mut self, query: &str) {
        let q = query.rsplit_once('/').map(|(_, a)| a).unwrap_or(query);
        if q.is_empty() {
            self.template_dropdown.items = self.all_template_items.clone();
        } else {
            let mut scored: Vec<(i64, TemplateItem)> = self
                .all_template_items
                .iter()
                .filter_map(|t| fuzzy_score(q, &t.template.name).map(|s| (s, t.clone())))
                .collect();
            scored.sort_by_key(|b| std::cmp::Reverse(b.0));
            self.template_dropdown.items = scored.into_iter().map(|(_, t)| t).collect();
        }
        if self.template_dropdown.selected >= self.template_dropdown.items.len() {
            self.template_dropdown.selected = self.template_dropdown.items.len().saturating_sub(1);
        }
        self.template_dropdown.visible = true;
    }

    pub fn open_file_dropdown(&mut self, query: &str) {
        self.active_modal = super::Modal::File;
        self.filter_file_dropdown(query);
    }

    pub fn filter_file_dropdown(&mut self, query: &str) {
        self.ensure_files_loaded();
        let q = query.rsplit_once('@').map(|(_, a)| a).unwrap_or(query);
        const LIMIT: usize = 64;
        if q.is_empty() {
            self.file_dropdown.items = self
                .all_files
                .iter()
                .take(LIMIT)
                .map(|f| LabeledItem {
                    label: f.clone(),
                    value: f.clone(),
                })
                .collect();
        } else {
            let mut scored: Vec<(i64, &str)> = Vec::new();
            for f in &self.all_files {
                if let Some(s) = fuzzy_score(q, f) {
                    scored.push((s, f.as_str()));
                }
            }
            scored.sort_by_key(|b| std::cmp::Reverse(b.0));
            self.file_dropdown.items = scored
                .into_iter()
                .take(LIMIT)
                .map(|(_, f)| LabeledItem {
                    label: f.to_string(),
                    value: f.to_string(),
                })
                .collect();
        }
        if self.file_dropdown.selected >= self.file_dropdown.items.len() {
            self.file_dropdown.selected = self.file_dropdown.items.len().saturating_sub(1);
        }
        self.file_dropdown.visible = true;
    }

    pub fn build_tree_from_index(
        &mut self,
        index: &[crate::session::types::TurnMeta],
        current_turn: &str,
    ) {
        let by_id: std::collections::HashMap<String, &crate::session::types::TurnMeta> =
            index.iter().map(|m| (m.id.clone(), m)).collect();
        let mut children: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for m in index {
            children
                .entry(m.parent_id.clone())
                .or_default()
                .push(m.id.clone());
        }

        let roots: Vec<String> = children.get("").cloned().unwrap_or_default();

        let mut items: Vec<TreeItem> = Vec::new();

        #[allow(clippy::too_many_arguments)]
        fn walk(
            turn_id: &str,
            depth: usize,
            ancestors: &mut Vec<bool>,
            is_last: bool,
            by_id: &std::collections::HashMap<String, &crate::session::types::TurnMeta>,
            children: &std::collections::HashMap<String, Vec<String>>,
            current_turn: &str,
            items: &mut Vec<TreeItem>,
        ) {
            let label = by_id
                .get(turn_id)
                .map(|m| m.label.clone())
                .unwrap_or_else(|| turn_id.to_string());
            let is_current = turn_id == current_turn;
            items.push(TreeItem {
                turn_id: turn_id.to_string(),
                label,
                depth,
                is_last,
                ancestors: ancestors.clone(),
                is_current,
            });

            if let Some(kids) = children.get(turn_id) {
                for (i, child_id) in kids.iter().enumerate() {
                    let child_last = i == kids.len() - 1;
                    ancestors.push(!is_last);
                    walk(
                        child_id,
                        depth + 1,
                        ancestors,
                        child_last,
                        by_id,
                        children,
                        current_turn,
                        items,
                    );
                    ancestors.pop();
                }
            }
        }

        let mut ancestors = Vec::new();
        for (i, root_id) in roots.iter().enumerate() {
            let last = i == roots.len() - 1;
            walk(
                root_id,
                0,
                &mut ancestors,
                last,
                &by_id,
                &children,
                current_turn,
                &mut items,
            );
        }

        self.tree_items = items;
    }

    pub fn show_tree(&mut self) {
        if self.tree_items.is_empty() {
            return;
        }
        let items = self.tree_items.clone();
        let mut selected = 0usize;
        for (i, item) in items.iter().enumerate() {
            if item.is_current {
                selected = i;
                break;
            }
        }
        self.tree_dropdown.items = items;
        self.tree_dropdown.selected = selected;
        self.tree_dropdown.visible = true;
        self.active_modal = super::Modal::Tree;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_dropdown_filters_fuzzy() {
        let mut s = ChatScreen::new(".".into(), vec![], true, true, true);
        s.open_command_dropdown();
        assert_eq!(s.command_dropdown.items.len(), COMMANDS.len());

        s.command_query = "copy".to_string();
        s.filter_command_dropdown();
        let labels: Vec<&str> = s
            .command_dropdown
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(labels, vec!["Copy Last Response"]);

        s.command_query = "toggle".to_string();
        s.filter_command_dropdown();
        let labels: Vec<String> = s
            .command_dropdown
            .items
            .iter()
            .map(|item| item.label.clone())
            .collect();
        assert_eq!(labels.len(), 3);
        assert!(labels.contains(&"Toggle Tool Results".to_string()));
        assert!(labels.contains(&"Toggle Subagent Calls".to_string()));
        assert!(labels.contains(&"Toggle Thinking".to_string()));
        assert_eq!(s.command_dropdown.selected, 0);

        s.command_query = "zzz".to_string();
        s.filter_command_dropdown();
        assert!(s.command_dropdown.items.is_empty());
    }
}
