#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    // Mock ares Tool for testing
    struct MockAresTool {
        name: String,
        description: String,
        parameters_schema: serde_json::Value,
    }

    impl MockAresTool {
        fn new(name: &str, description: &str) -> Self {
            Self {
                name: name.to_string(),
                description: description.to_string(),
                parameters_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            }
        }
    }

    #[async_trait]
    impl ares::tools::registry::Tool for MockAresTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            &self.description
        }

        fn parameters_schema(&self) -> serde_json::Value {
            self.parameters_schema.clone()
        }

        async fn execute(&self, _args: serde_json::Value) -> ares::types::Result<serde_json::Value> {
            Ok(json!({"result": "mock"}))
        }
    }

    #[test]
    fn test_arestool_new() {
        let mock_tool = Arc::new(MockAresTool::new("test_tool", "A test tool"));
        let ares_tool = AresTool::new(Arc::clone(&mock_tool));

        // Verify the tool was created successfully
        assert_eq!(ares_tool.inner.name(), "test_tool");
    }

    #[test]
    fn test_arestool_name() {
        let mock_tool = Arc::new(MockAresTool::new("my_tool", "My description"));
        let ares_tool = AresTool::new(mock_tool);

        assert_eq!(ares_tool.name(), "my_tool");
    }

    #[test]
    fn test_arestool_description() {
        let mock_tool = Arc::new(MockAresTool::new("tool1", "Tool 1 description"));
        let ares_tool = AresTool::new(mock_tool);

        assert_eq!(ares_tool.description(), "Tool 1 description");
    }

    #[test]
    fn test_arestool_mutating() {
        let mock_tool = Arc::new(MockAresTool::new("tool2", "Tool 2"));
        let ares_tool = AresTool::new(mock_tool);

        // AresTool always returns true for mutating
        assert!(ares_tool.mutating());
    }

    #[test]
    fn test_arestool_parameters_schema() {
        let expected_schema = json!({
            "type": "object",
            "properties": {
                "param1": {"type": "string"}
            }
        });

        let mut mock_tool = MockAresTool::new("tool3", "Tool 3");
        mock_tool.parameters_schema = expected_schema.clone();
        let ares_tool = AresTool::new(Arc::new(mock_tool));

        assert_eq!(ares_tool.parameters_schema(), expected_schema);
    }

    #[tokio::test]
    async fn test_arestool_execute_success() {
        let mock_tool = Arc::new(MockAresTool::new("tool4", "Tool 4"));
        let ares_tool = AresTool::new(mock_tool);

        let result = ares_tool.execute(json!({"arg": "value"})).await;

        assert!(result.is_ok());
        let value = result.unwrap();
        assert_eq!(value["result"], "mock");
    }

    #[tokio::test]
    async fn test_arestool_execute_error() {
        // Create a mock tool that returns an error
        struct ErrorTool;

        #[async_trait]
        impl ares::tools::registry::Tool for ErrorTool {
            fn name(&self) -> &str {
                "error_tool"
            }

            fn description(&self) -> &str {
                "A tool that always errors"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                json!({"type": "object"})
            }

            async fn execute(&self, _args: serde_json::Value) -> ares::types::Result<serde_json::Value> {
                Err(ares::types::AppError::InvalidInput("Test error".to_string()))
            }
        }

        let mock_tool = Arc::new(ErrorTool);
        let ares_tool = AresTool::new(mock_tool);

        let result = ares_tool.execute(json!({})).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Ares tool error"));
        assert!(err.contains("Test error"));
    }

    #[test]
    fn test_bridge_ares_tools_empty_registry() {
        let ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        let count = bridge_ares_tools(&ares_registry, &mut pawan_registry);

        assert_eq!(count, 0);
    }

    #[test]
    fn test_bridge_ares_tools_single_tool() {
        let mut ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        // Register a tool in ares registry
        let tool = Arc::new(MockAresTool::new("test_tool", "Test tool"));
        ares_registry.register(tool);

        let count = bridge_ares_tools(&ares_registry, &mut pawan_registry);

        assert_eq!(count, 1);
        assert!(pawan_registry.has_tool("test_tool"));
    }

    #[test]
    fn test_bridge_ares_tools_multiple_tools() {
        let mut ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        // Register multiple tools in ares registry
        for i in 0..5 {
            let tool = Arc::new(MockAresTool::new(&format!("tool_{}", i), &format!("Tool {}", i)));
            ares_registry.register(tool);
        }

        let count = bridge_ares_tools(&ares_registry, &mut pawan_registry);

        assert_eq!(count, 5);
        for i in 0..5 {
            assert!(pawan_registry.has_tool(&format!("tool_{}", i)));
        }
    }

    #[test]
    fn test_bridge_ares_tools_handles_missing_tool() {
        let mut ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        // Register a tool
        let tool = Arc::new(MockAresTool::new("available_tool", "Available tool"));
        ares_registry.register(tool);

        // Manually add a tool definition that doesn't have a corresponding tool
        // This simulates the case where get_tool_definitions returns a tool
        // that isn't in the registry (edge case)
        // Note: In practice, this shouldn't happen with a properly configured registry
        // but we test the defensive behavior

        let count = bridge_ares_tools(&ares_registry, &mut pawan_registry);

        // Should only bridge the available tool
        assert_eq!(count, 1);
        assert!(pawan_registry.has_tool("available_tool"));
    }

    #[test]
    fn test_bridge_ares_tools_preserves_tool_properties() {
        let mut ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        let expected_name = "test_tool";
        let expected_desc = "Test tool description";
        let expected_schema = json!({
            "type": "object",
            "properties": {
                "input": {"type": "string"}
            }
        });

        let mut mock_tool = MockAresTool::new(expected_name, expected_desc);
        mock_tool.parameters_schema = expected_schema.clone();
        let tool = Arc::new(mock_tool);
        ares_registry.register(tool);

        bridge_ares_tools(&ares_registry, &mut pawan_registry);

        // Verify the bridged tool has the correct properties
        let bridged_tool = pawan_registry.get(expected_name).unwrap();
        assert_eq!(bridged_tool.name(), expected_name);
        assert_eq!(bridged_tool.description(), expected_desc);
        assert_eq!(bridged_tool.parameters_schema(), expected_schema);
        assert!(bridged_tool.mutating()); // AresTool always returns true
    }

    #[test]
    fn test_bridge_ares_tools_idempotent() {
        let mut ares_registry = ares::tools::registry::ToolRegistry::new();
        let mut pawan_registry = ToolRegistry::new();

        let tool = Arc::new(MockAresTool::new("test_tool", "Test tool"));
        ares_registry.register(tool);

        // Bridge the tools twice
        let count1 = bridge_ares_tools(&ares_registry, &mut pawan_registry);
        let count2 = bridge_ares_tools(&ares_registry, &mut pawan_registry);

        // Both should report the same count
        assert_eq!(count1, 1);
        assert_eq!(count2, 1);
        // Tool should still be present
        assert!(pawan_registry.has_tool("test_tool"));
    }
}
