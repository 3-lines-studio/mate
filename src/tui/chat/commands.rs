use super::ChatScreen;

impl ChatScreen {
    pub fn copy_last_response(&mut self) {
        for msg in self.messages.iter().rev() {
            if msg.role == "assistant" {
                let text = super::super::chat_handlers::assemble_message_prose(msg);
                if !text.is_empty() {
                    match arboard::Clipboard::new() {
                        Ok(mut clipboard) => {
                            if clipboard.set_text(&text).is_ok() {
                                self.add_message("assistant", "Copied last response to clipboard");
                            } else {
                                self.add_message("error", "Failed to copy to clipboard");
                            }
                        }
                        Err(e) => {
                            self.add_message("error", &format!("Clipboard error: {}", e));
                        }
                    }
                }
                return;
            }
        }
        self.add_message("error", "No assistant response to copy");
    }

    pub fn export_markdown(&mut self) {
        let mut sb = String::new();
        sb.push_str(&format!("# Mate Session: {}\n\n", self.session_name));
        sb.push_str(&format!("**Model:** {}  \n", self.model_name));
        sb.push_str(&format!(
            "**Date:** {}  \n\n---\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        ));

        for msg in &self.messages {
            match msg.role.as_str() {
                "user" => {
                    sb.push_str(&format!("### You\n\n{}\n\n", msg.content));
                }
                "assistant" => {
                    let text = super::super::chat_handlers::assemble_message_full_text(msg);
                    sb.push_str(&format!("### Mate\n\n{}\n\n", text));
                }
                "error" => {
                    sb.push_str(&format!("### Error\n\n{}\n\n", msg.content));
                }
                _ => {}
            }
        }

        let filename = format!(
            "mate-export-{}.md",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let path = std::path::PathBuf::from(&self.cwd).join(&filename);
        if let Err(e) = std::fs::write(&path, sb) {
            self.add_message("error", &format!("Export failed: {}", e));
        } else {
            self.add_message("assistant", &format!("Exported to {}", filename));
        }
    }
}
