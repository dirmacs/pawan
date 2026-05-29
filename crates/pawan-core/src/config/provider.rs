use serde::{Deserialize, Serialize};

/// LLM Provider type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    /// NVIDIA API (build.nvidia.com) - default
    #[default]
    Nvidia,
    /// Local Ollama instance
    Ollama,
    /// OpenAI-compatible API
    OpenAI,
    /// MLX LM server (Apple Silicon native, mlx_lm.server) — auto-routes to localhost:8080
    Mlx,
}
