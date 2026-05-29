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

    /// Load available models — live fetch with fallback.
    pub(crate) fn load_available_models(&mut self) {
        self.model_picker.models = super::model_catalog::default_models();
    }
}
