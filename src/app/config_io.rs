use super::App;

impl App {
    pub(super) fn update_config_file<F>(&mut self, error_context: &str, update: F) -> bool
    where
        F: FnOnce(&str) -> String,
    {
        #[cfg(test)]
        if std::env::var_os(crate::config::CONFIG_PATH_ENV_VAR).is_none() {
            return false;
        }

        let path = crate::config::config_path();
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                crate::logging::config_write_failed(&path, error_context, &err.to_string());
                self.state.config_diagnostic =
                    Some(format!("failed to save {error_context}: {err}"));
                self.config_diagnostic_deadline =
                    Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                return false;
            }
        }

        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let new_content = update(&content);
        if let Err(err) = std::fs::write(&path, new_content) {
            crate::logging::config_write_failed(&path, error_context, &err.to_string());
            self.state.config_diagnostic = Some(format!("failed to save {error_context}: {err}"));
            self.config_diagnostic_deadline =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
            return false;
        }

        true
    }

    /// Declare or undeclare the workspace's directory in `[[repos]]`. The
    /// pinned path is the workspace's own directory; the focused pane's cwd
    /// stands in for an unpinned workspace, matching the pin action.
    pub(super) fn toggle_declared_repo(&mut self, ws_idx: usize) {
        let path = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.pinned_path.clone())
            .or_else(|| self.focused_pane_cwd_in_workspace(ws_idx));
        let Some(path) = path else {
            return;
        };

        let mut declared = false;
        if !self.update_config_file("declared repos", |content| {
            let (updated, now_declared) = crate::config::toggle_repo_path(content, &path);
            declared = now_declared;
            updated
        }) {
            return;
        }
        self.apply_config_from_disk(false);

        let previous_toast = self.state.toast.clone();
        self.state.toast = Some(crate::app::state::ToastNotification {
            // The same neutral informational toast the pin action uses.
            kind: crate::app::state::ToastKind::UpdateInstalled,
            title: if declared {
                "declared repo".to_string()
            } else {
                "undeclared repo".to_string()
            },
            context: path.display().to_string(),
            position: None,
            target: None,
        });
        self.sync_toast_deadline(previous_toast);
    }

    pub(super) fn mark_onboarding_complete(&mut self) {
        self.update_config_file("onboarding setting", |content| {
            crate::config::upsert_top_level_bool(content, "onboarding", false)
        });
    }

    pub(super) fn save_theme(&mut self, name: &str) {
        if self.update_config_file("theme", |content| {
            let content = crate::config::upsert_section_value(
                content,
                "theme",
                "name",
                &format!("\"{name}\""),
            );
            crate::config::upsert_section_bool(&content, "theme", "auto_switch", false)
        }) {
            self.apply_config_from_disk(false);
        }
    }

    pub(super) fn save_status_indicators(&mut self, style: crate::config::StatusIndicatorStyle) {
        if self.update_config_file("status indicators", |content| {
            crate::config::upsert_section_value(
                content,
                "ui",
                "status_indicators",
                &format!("\"{}\"", style.as_str()),
            )
        }) {
            self.apply_config_from_disk(false);
        }
    }

    pub(super) fn save_sound(&mut self, enabled: bool) {
        if self.update_config_file("sound setting", |content| {
            crate::config::upsert_section_bool(content, "ui.sound", "enabled", enabled)
        }) {
            self.apply_config_from_disk(false);
        }
    }

    pub(super) fn save_toast_delivery(&mut self, delivery: crate::config::ToastDelivery) {
        let value = match delivery {
            crate::config::ToastDelivery::Off => "\"off\"",
            crate::config::ToastDelivery::Herdr => "\"herdr\"",
            crate::config::ToastDelivery::Terminal => "\"terminal\"",
            crate::config::ToastDelivery::System => "\"system\"",
        };
        if self.update_config_file("toast setting", |content| {
            let content =
                crate::config::upsert_section_value(content, "ui.toast", "delivery", value);
            crate::config::remove_section_key(&content, "ui.toast", "enabled")
        }) {
            self.apply_config_from_disk(false);
        }
    }

    pub(super) fn save_agent_border_labels(&mut self, enabled: bool) {
        if self.update_config_file("agent border labels", |content| {
            crate::config::upsert_section_bool(
                content,
                "ui",
                "show_agent_labels_on_pane_borders",
                enabled,
            )
        }) {
            self.apply_config_from_disk(false);
        }
    }

    pub(super) fn save_agent_panel_sort(&mut self, sort: crate::app::state::AgentPanelSort) {
        let value = match sort {
            crate::app::state::AgentPanelSort::Spaces => {
                crate::config::AgentPanelSortConfig::Spaces.as_str()
            }
            crate::app::state::AgentPanelSort::Priority => {
                crate::config::AgentPanelSortConfig::Priority.as_str()
            }
        };
        if self.update_config_file("agent panel sort", |content| {
            crate::config::upsert_section_value(
                content,
                "ui",
                "agent_panel_sort",
                &format!("\"{value}\""),
            )
        }) {
            self.apply_config_from_disk(false);
        }
    }
}
