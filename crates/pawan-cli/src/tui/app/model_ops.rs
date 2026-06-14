//! Model switching and catalog loading.

use pawan::agent::Role;

use super::state::App;
use super::types::*;

impl<'a> App<'a> {
    /// Switch the active model (UI + agent task).
    pub(crate) fn switch_model(&mut self, model_id: String) {
        self.model_name = model_id.clone();
        self.status = format!("Model → {}", model_id);
        self.messages.push(DisplayMessage::new_text(
            Role::System,
            format!("Switched to model: {}", model_id),
        ));
        let _ = self.cmd_tx.send(AgentCommand::SwitchModel(model_id));
    }

    /// Ensure the model picker has a catalog.
    ///
    /// Live NVIDIA `/v1/models` results arrive asynchronously in `poll_model_fetch`.
    /// Do not replace that live catalog when `/model` opens; only seed the curated
    /// fallback while the live fetch has not produced anything yet.
    pub(crate) fn load_available_models(&mut self) {
        if self.model_picker.models.is_empty() {
            self.model_picker.models = super::model_catalog::default_models();
        }
    }
}
