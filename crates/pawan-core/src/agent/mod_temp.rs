    /// Get the configuration
    ///
    /// Returns a reference to the current PawanConfig
    pub fn config(&self) -> &PawanConfig {
        &self.config
    }

    /// Execute the agent with callback functions for streaming output
    ///
    /// # Arguments
    ///
    /// * `user_prompt` - The user's prompt or query
    /// * `on_token` - Optional callback invoked for each token as it's generated
    /// * `on_tool` - Optional callback invoked when a tool is called
    /// * `on_tool_start` - Optional callback invoked when a tool starts executing
    ///
    /// # Returns
    ///
    /// An AgentResponse containing the final response, tool calls made, and usage statistics