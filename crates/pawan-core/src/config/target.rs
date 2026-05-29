use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for a target project
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Configuration for a target project
///
/// This struct represents configuration for a specific target project that Pawan
/// can work with. It includes the project path and description.
pub struct TargetConfig {
    /// Path to the project root
    pub path: PathBuf,

    /// Description of the project
    pub description: String,
}
