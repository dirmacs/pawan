//! Pawan CLI Entry Point
//!
//! Provides the main command-line interface for Pawan with subcommands:
//! - `pawan` - Interactive TUI mode (default)
//! - `pawan heal` - Auto-fix compilation errors, warnings, and tests
//! - `pawan task <description>` - Execute a coding task
//! - `pawan commit` - Generate commit message
//! - `pawan improve <what>` - Improve code (docs, refactor, etc.)

#[cfg(feature = "tui")]
mod tui;

use clap::{Parser, Subcommand};
use owo_colors::OwoColorize;
use pawan::{agent::PawanAgent, config::PawanConfig, healing::Healer, PawanError, Result};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pawan")]
#[command(
    author,
    version,
    about = "Pawan (पवन) - Self-healing, self-improving CLI coding agent"
)]
#[command(long_about = r#"
Pawan is a powerful CLI coding agent that can:
  • Automatically fix compilation errors and warnings
  • Execute complex coding tasks
  • Generate documentation and commit messages
  • Work on any Rust project including itself

Examples:
  pawan              # Interactive TUI mode
  pawan heal         # Auto-fix all issues
  pawan task "add input validation to CreateAgentRequest"
  pawan commit       # Generate commit message
  pawan improve docs # Generate missing documentation
"#)]
struct Cli {
    /// Path to workspace root (defaults to current directory)
    #[arg(short, long, global = true)]
    workspace: Option<PathBuf>,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Model to use (overrides config)
    #[arg(short, long, global = true)]
    model: Option<String>,

    /// Dry run mode (show what would be done without making changes)
    #[arg(long, global = true)]
    dry_run: bool,

    /// Disable TUI and use simple CLI mode
    #[arg(long, global = true)]
    no_tui: bool,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Self-heal the project (fix errors, warnings, tests)
    Heal {
        /// Only fix compilation errors
        #[arg(long)]
        errors_only: bool,

        /// Only fix clippy warnings
        #[arg(long)]
        warnings_only: bool,

        /// Only fix failing tests
        #[arg(long)]
        tests_only: bool,

        /// Auto-commit fixes
        #[arg(long)]
        commit: bool,
    },

    /// Execute a coding task
    Task {
        /// Description of the task to execute
        description: String,
    },

    /// Generate a commit message for current changes
    Commit {
        /// Include body with detailed changes
        #[arg(long)]
        with_body: bool,
    },

    /// Improve the codebase
    Improve {
        /// What to improve: docs, refactor, tests, all
        target: String,

        /// Specific file or module to improve
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Show project status (errors, warnings, test failures)
    Status,

    /// Interactive chat mode (same as running without subcommand)
    Chat {
        /// Resume a saved session by ID
        #[arg(long)]
        resume: Option<String>,
    },

    /// List saved sessions
    Sessions,

    /// MCP server management
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },

    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Headless single-prompt execution (for scripting and orchestration)
    Run {
        /// The prompt to execute
        prompt: Option<String>,

        /// Read prompt from file instead of argument
        #[arg(short = 'f', long)]
        file: Option<PathBuf>,

        /// Output format: text (default), json
        #[arg(short, long, default_value = "text")]
        output: String,

        /// Maximum time in seconds before aborting
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// Maximum tool iterations
        #[arg(long)]
        max_iterations: Option<usize>,

        /// Save session after completion
        #[arg(long)]
        save: bool,
    },
}

#[derive(Subcommand)]
enum McpAction {
    /// List connected MCP servers and their tools
    List,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show the resolved configuration
    Show,
    /// Generate a pawan.toml template
    Init,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("{} {}", "Error:".red().bold(), e);
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    // Auto-load .env file if present (silent on missing)
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    // Determine workspace root
    let workspace = cli
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load configuration
    let mut config = PawanConfig::load(cli.config.as_ref())?;

    // Apply CLI overrides
    if let Some(model) = cli.model {
        config.model = model;
    }
    if cli.dry_run {
        config.dry_run = true;
    }

    if cli.verbose {
        println!("{} {}", "Workspace:".cyan().bold(), workspace.display());
        println!("{} {}", "Model:".cyan().bold(), config.model);
        if config.dry_run {
            println!("{}", "Dry-run mode enabled".yellow());
        }
    }

    match cli.command {
        None | Some(Commands::Chat { resume: None }) => {
            run_interactive(config, workspace, cli.no_tui, None).await
        }
        Some(Commands::Chat { resume: Some(id) }) => {
            run_interactive(config, workspace, cli.no_tui, Some(id)).await
        }
        Some(Commands::Sessions) => run_sessions().await,
        Some(Commands::Mcp { action }) => match action {
            McpAction::List => run_mcp_list(config).await,
        },
        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => run_config_show(config),
            ConfigAction::Init => run_config_init(),
        },
        Some(Commands::Heal {
            errors_only,
            warnings_only,
            tests_only,
            commit,
        }) => {
            run_heal(
                config,
                workspace,
                errors_only,
                warnings_only,
                tests_only,
                commit,
                cli.verbose,
            )
            .await
        }
        Some(Commands::Task { description }) => {
            run_task(config, workspace, &description, cli.verbose).await
        }
        Some(Commands::Commit { with_body }) => run_commit(config, workspace, with_body).await,
        Some(Commands::Improve { target, file }) => {
            run_improve(config, workspace, &target, file, cli.verbose).await
        }
        Some(Commands::Status) => run_status(config, workspace).await,
        Some(Commands::Run {
            prompt,
            file,
            output,
            timeout,
            max_iterations,
            save,
        }) => {
            run_headless(
                config,
                workspace,
                prompt,
                file,
                &output,
                timeout,
                max_iterations,
                save,
                cli.verbose,
            )
            .await
        }
    }
}

/// Run interactive mode
async fn run_interactive(
    config: PawanConfig,
    workspace: PathBuf,
    no_tui: bool,
    resume_id: Option<String>,
) -> Result<()> {
    let mut agent = PawanAgent::new(config.clone(), workspace);

    #[cfg(feature = "mcp")]
    setup_mcp_tools(&mut agent, &config).await;

    if let Some(id) = resume_id {
        agent.resume_session(&id)?;
        if !no_tui {
            eprintln!("Resumed session: {}", id);
        }
    }

    #[cfg(feature = "tui")]
    {
        if no_tui {
            crate::tui::run_simple(agent).await
        } else {
            crate::tui::run_tui(agent, config.tui).await
        }
    }

    #[cfg(not(feature = "tui"))]
    {
        let _ = no_tui;
        run_simple_cli(agent).await
    }
}

/// Run self-healing
async fn run_heal(
    mut config: PawanConfig,
    workspace: PathBuf,
    errors_only: bool,
    warnings_only: bool,
    tests_only: bool,
    commit: bool,
    verbose: bool,
) -> Result<()> {
    // Adjust healing config based on flags
    if errors_only {
        config.healing.fix_warnings = false;
        config.healing.fix_tests = false;
    }
    if warnings_only {
        config.healing.fix_errors = false;
        config.healing.fix_tests = false;
    }
    if tests_only {
        config.healing.fix_errors = false;
        config.healing.fix_warnings = false;
    }
    if commit {
        config.healing.auto_commit = true;
    }

    println!("{}", "Pawan Self-Healing Mode".green().bold());
    println!("{}", "═".repeat(40).dimmed());

    // First, check current status
    let healer = Healer::new(workspace.clone(), config.healing.clone());
    let (errors, warnings, failed_tests) = healer.count_issues().await?;

    println!(
        "\n{} {} errors, {} warnings, {} failed tests",
        "Found:".cyan().bold(),
        errors.to_string().red(),
        warnings.to_string().yellow(),
        failed_tests.to_string().red()
    );

    if errors == 0 && warnings == 0 && failed_tests == 0 {
        println!("\n{}", "✓ Project is healthy!".green().bold());
        return Ok(());
    }

    // Create agent and start healing
    let mut agent = PawanAgent::new(config.clone(), workspace);

    println!("\n{}", "Starting healing process...".cyan());

    let response = agent.heal().await?;

    println!("\n{}", "═".repeat(40).dimmed());
    println!("{}", response.content);

    if verbose && !response.tool_calls.is_empty() {
        println!("\n{}", "Tool calls made:".dimmed());
        for tc in &response.tool_calls {
            let status_str = if tc.success { "✓" } else { "✗" };
            if tc.success {
                println!(
                    "  {} {} ({}ms)",
                    status_str.green(),
                    tc.name.cyan(),
                    tc.duration_ms
                );
            } else {
                println!(
                    "  {} {} ({}ms)",
                    status_str.red(),
                    tc.name.cyan(),
                    tc.duration_ms
                );
            }
        }
    }

    // Check final status
    let (final_errors, final_warnings, final_tests) = healer.count_issues().await?;

    println!("\n{}", "Final Status:".cyan().bold());
    print!("  Errors: {} → ", errors.to_string().dimmed());
    if final_errors < errors {
        println!("{}", final_errors.to_string().green());
    } else {
        println!("{}", final_errors);
    }
    print!("  Warnings: {} → ", warnings.to_string().dimmed());
    if final_warnings < warnings {
        println!("{}", final_warnings.to_string().green());
    } else {
        println!("{}", final_warnings);
    }
    print!("  Failed Tests: {} → ", failed_tests.to_string().dimmed());
    if final_tests < failed_tests {
        println!("{}", final_tests.to_string().green());
    } else {
        println!("{}", final_tests);
    }

    Ok(())
}

/// Run a specific task
async fn run_task(
    config: PawanConfig,
    workspace: PathBuf,
    description: &str,
    verbose: bool,
) -> Result<()> {
    println!("{}", "Pawan Task Mode".green().bold());
    println!("{}", "═".repeat(40).dimmed());
    println!("{} {}", "Task:".cyan().bold(), description);
    println!();

    let config_ref = config.clone();
    let mut agent = PawanAgent::new(config, workspace);

    #[cfg(feature = "mcp")]
    setup_mcp_tools(&mut agent, &config_ref).await;

    let response = agent.task(description).await?;

    println!("{}", response.content);

    if verbose && !response.tool_calls.is_empty() {
        println!("\n{}", "Tool calls made:".dimmed());
        for tc in &response.tool_calls {
            let status_str = if tc.success { "✓" } else { "✗" };
            if tc.success {
                println!(
                    "  {} {} ({}ms)",
                    status_str.green(),
                    tc.name.cyan(),
                    tc.duration_ms
                );
            } else {
                println!(
                    "  {} {} ({}ms)",
                    status_str.red(),
                    tc.name.cyan(),
                    tc.duration_ms
                );
            }
        }
    }

    Ok(())
}

/// Generate commit message
async fn run_commit(config: PawanConfig, workspace: PathBuf, _with_body: bool) -> Result<()> {
    println!("{}", "Generating commit message...".cyan());

    let mut agent = PawanAgent::new(config, workspace);
    let message = agent.generate_commit_message().await?;

    println!("\n{}", "Suggested commit message:".green().bold());
    println!("{}", "─".repeat(40).dimmed());
    println!("{}", message);
    println!("{}", "─".repeat(40).dimmed());

    // Ask if they want to use it
    use dialoguer::Confirm;

    if Confirm::new()
        .with_prompt("Use this commit message?")
        .default(false)
        .interact()
        .unwrap_or(false)
    {
        // Stage and commit
        let output = std::process::Command::new("git")
            .args(["commit", "-m", &message])
            .output()
            .map_err(PawanError::Io)?;

        if output.status.success() {
            println!("{}", "✓ Committed successfully!".green());
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            println!("{} {}", "Commit failed:".red(), stderr);
        }
    }

    Ok(())
}

/// Run improvement task
async fn run_improve(
    config: PawanConfig,
    workspace: PathBuf,
    target: &str,
    file: Option<PathBuf>,
    verbose: bool,
) -> Result<()> {
    let description = match target.to_lowercase().as_str() {
        "docs" | "documentation" => {
            if let Some(ref f) = file {
                format!(
                    "Generate comprehensive documentation for all public items in {}. \
                     Add module-level docs, function docs, struct/enum docs with examples where helpful.",
                    f.display()
                )
            } else {
                "Generate comprehensive documentation for all public items that are missing docs. \
                 Focus on module-level docs, function docs, and struct/enum docs."
                    .to_string()
            }
        }
        "refactor" => {
            if let Some(ref f) = file {
                format!(
                    "Refactor {} to improve code quality. Look for: \
                     - Long functions that can be split \
                     - Code duplication that can be extracted \
                     - Complex conditionals that can be simplified \
                     - Better naming opportunities",
                    f.display()
                )
            } else {
                "Analyze the codebase and suggest refactoring opportunities. \
                 Look for code duplication, overly complex functions, and naming improvements."
                    .to_string()
            }
        }
        "tests" => {
            if let Some(ref f) = file {
                format!(
                    "Add comprehensive unit tests for {}. \
                     Cover edge cases, error conditions, and typical use cases.",
                    f.display()
                )
            } else {
                "Identify areas with insufficient test coverage and add tests. \
                 Focus on critical business logic and edge cases."
                    .to_string()
            }
        }
        "all" => "Improve the overall code quality: \
             1. Fix any clippy warnings \
             2. Add missing documentation \
             3. Suggest and apply refactoring improvements \
             4. Add missing tests for uncovered code"
            .to_string(),
        _ => {
            return Err(PawanError::Config(format!(
                "Unknown improvement target: {}. Use: docs, refactor, tests, or all",
                target
            )));
        }
    };

    run_task(config, workspace, &description, verbose).await
}

/// Show project status
async fn run_status(config: PawanConfig, workspace: PathBuf) -> Result<()> {
    println!("{}", "Pawan Project Status".green().bold());
    println!("{}", "═".repeat(40).dimmed());

    let healer = Healer::new(workspace.clone(), config.healing);

    // Get diagnostics
    println!("\n{}", "Checking compilation...".dimmed());
    let diagnostics = healer.get_diagnostics().await?;

    let errors: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == pawan::healing::DiagnosticKind::Error)
        .collect();
    let warnings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.kind == pawan::healing::DiagnosticKind::Warning)
        .collect();

    println!("{}", "Checking tests...".dimmed());
    let failed_tests = healer.get_failed_tests().await?;

    // Print summary
    println!("\n{}", "Summary:".cyan().bold());

    if errors.is_empty() {
        println!("  {} No compilation errors", "✓".green());
    } else {
        println!(
            "  {} {} compilation error(s)",
            "✗".red(),
            errors.len().to_string().red().bold()
        );
        for err in errors.iter().take(5) {
            println!(
                "    {} {}",
                "→".dimmed(),
                err.message.chars().take(60).collect::<String>()
            );
        }
        if errors.len() > 5 {
            println!("    {} ...and {} more", "→".dimmed(), errors.len() - 5);
        }
    }

    if warnings.is_empty() {
        println!("  {} No warnings", "✓".green());
    } else {
        println!(
            "  {} {} warning(s)",
            "⚠".yellow(),
            warnings.len().to_string().yellow().bold()
        );
    }

    if failed_tests.is_empty() {
        println!("  {} All tests passing", "✓".green());
    } else {
        println!(
            "  {} {} test(s) failing",
            "✗".red(),
            failed_tests.len().to_string().red().bold()
        );
        for test in failed_tests.iter().take(5) {
            println!("    {} {}", "→".dimmed(), test.name);
        }
        if failed_tests.len() > 5 {
            println!(
                "    {} ...and {} more",
                "→".dimmed(),
                failed_tests.len() - 5
            );
        }
    }

    // Overall health
    println!();
    if errors.is_empty() && warnings.is_empty() && failed_tests.is_empty() {
        println!("{}", "✓ Project is healthy!".green().bold());
    } else {
        println!(
            "{}",
            "Run 'pawan heal' to automatically fix issues".yellow()
        );
    }

    Ok(())
}

/// List saved sessions
async fn run_sessions() -> Result<()> {
    use pawan::agent::session::Session;

    let sessions = Session::list()?;

    if sessions.is_empty() {
        println!("{}", "No saved sessions.".dimmed());
        return Ok(());
    }

    println!("{}", "Saved Sessions".green().bold());
    println!("{}", "═".repeat(60).dimmed());
    println!(
        "  {:<10} {:<30} {:<6} {}",
        "ID".cyan(),
        "Model".cyan(),
        "Msgs".cyan(),
        "Updated".cyan()
    );
    println!("{}", "─".repeat(60).dimmed());

    for s in &sessions {
        let model_short = if s.model.len() > 28 {
            format!("...{}", &s.model[s.model.len() - 25..])
        } else {
            s.model.clone()
        };
        let updated = &s.updated_at[..19]; // trim timezone
        println!(
            "  {:<10} {:<30} {:<6} {}",
            s.id, model_short, s.message_count, updated
        );
    }

    println!("\n{}", "Resume with: pawan chat --resume <ID>".dimmed());

    Ok(())
}

/// Set up MCP tools on an agent (if configured)
#[cfg(feature = "mcp")]
async fn setup_mcp_tools(agent: &mut PawanAgent, config: &PawanConfig) {
    use pawan_mcp::{McpManager, McpServerConfig};

    if config.mcp.is_empty() {
        return;
    }

    let configs: Vec<McpServerConfig> = config
        .mcp
        .iter()
        .map(|(name, entry)| McpServerConfig {
            name: name.clone(),
            command: entry.command.clone(),
            args: entry.args.clone(),
            env: entry.env.clone(),
            enabled: entry.enabled,
        })
        .collect();

    match McpManager::connect(&configs).await {
        Ok(manager) => {
            let count = manager.register_tools(agent.tools_mut());
            if count > 0 {
                eprintln!("Loaded {} MCP tools", count);
            }
            // Leak manager to keep connections alive for the process lifetime
            Box::leak(Box::new(manager));
        }
        Err(e) => {
            eprintln!("Warning: MCP setup failed: {}", e);
        }
    }
}

/// Show resolved configuration
fn run_config_show(config: PawanConfig) -> Result<()> {
    let toml_str = toml::to_string_pretty(&config)
        .map_err(|e| PawanError::Config(format!("Failed to serialize config: {}", e)))?;
    println!("{}", toml_str);
    Ok(())
}

/// Generate pawan.toml template
fn run_config_init() -> Result<()> {
    let path = std::path::Path::new("pawan.toml");
    if path.exists() {
        return Err(PawanError::Config(
            "pawan.toml already exists. Remove it first.".into(),
        ));
    }

    let template = r#"# Pawan configuration
# See: https://github.com/dirmacs/pawan

# LLM provider: nvidia, ollama, openai
# provider = "nvidia"

# Model to use (provider-specific ID)
model = "mistralai/devstral-2-123b-instruct-2512"

# Generation parameters
temperature = 0.6
# top_p = 0.95
# max_tokens = 8192

# Self-healing settings
[healing]
fix_errors = true
fix_warnings = true
fix_tests = true
auto_commit = false

# MCP servers (uncomment to enable)
# [mcp.daedra]
# command = "daedra"
# args = ["serve", "--transport", "stdio", "--quiet"]
"#;

    std::fs::write(path, template).map_err(PawanError::Io)?;
    println!("{} Created pawan.toml", "✓".green());
    Ok(())
}

/// List connected MCP servers and their tools
async fn run_mcp_list(config: PawanConfig) -> Result<()> {
    #[cfg(feature = "mcp")]
    {
        use pawan_mcp::{McpManager, McpServerConfig};

        if config.mcp.is_empty() {
            println!("{}", "No MCP servers configured in pawan.toml.".dimmed());
            println!(
                "\n{}",
                "Add servers under [mcp.<name>] with command and args.".dimmed()
            );
            return Ok(());
        }

        let configs: Vec<McpServerConfig> = config
            .mcp
            .iter()
            .map(|(name, entry)| McpServerConfig {
                name: name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                env: entry.env.clone(),
                enabled: entry.enabled,
            })
            .collect();

        println!("{}", "Connecting to MCP servers...".dimmed());
        let manager = McpManager::connect(&configs).await?;

        println!("\n{}", "MCP Servers".green().bold());
        println!("{}", "═".repeat(50).dimmed());

        for (name, count) in manager.summary() {
            println!("  {} {} ({} tools)", "●".green(), name.cyan(), count);
        }

        // Register to get full tool list
        let mut registry = pawan::tools::ToolRegistry::new();
        let total = manager.register_tools(&mut registry);
        println!("\n{}", "Available Tools".green().bold());
        println!("{}", "─".repeat(50).dimmed());
        for name in registry.tool_names() {
            println!("  {}", name);
        }
        println!("\n  Total: {} MCP tools", total);
    }

    #[cfg(not(feature = "mcp"))]
    {
        let _ = config;
        println!("MCP support not enabled. Build with --features mcp");
    }

    Ok(())
}

/// Headless single-prompt execution (replaces oh-my-opencode `run`)
#[allow(clippy::too_many_arguments)]
async fn run_headless(
    mut config: PawanConfig,
    workspace: PathBuf,
    prompt: Option<String>,
    file: Option<PathBuf>,
    output_format: &str,
    timeout_secs: u64,
    max_iterations: Option<usize>,
    save_session: bool,
    verbose: bool,
) -> Result<()> {
    // Resolve prompt from argument or file
    let prompt_text = match (prompt, file) {
        (Some(p), _) => p,
        (None, Some(f)) => std::fs::read_to_string(&f).map_err(|e| {
            PawanError::Config(format!("Failed to read prompt file {}: {}", f.display(), e))
        })?,
        (None, None) => {
            return Err(PawanError::Config(
                "Either a prompt argument or --file is required for `run`".into(),
            ));
        }
    };

    if let Some(max_iter) = max_iterations {
        config.max_tool_iterations = max_iter;
    }

    let config_ref = config.clone();
    let mut agent = PawanAgent::new(config, workspace);

    #[cfg(feature = "mcp")]
    setup_mcp_tools(&mut agent, &config_ref).await;

    if verbose && output_format != "json" {
        eprintln!("Model: {}", agent.config().model);
        eprintln!("Prompt: {}", &prompt_text[..prompt_text.len().min(100)]);
    }

    // Execute with timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        agent.execute(&prompt_text),
    )
    .await;

    let response = match result {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => {
            if output_format == "json" {
                let err_json = serde_json::json!({
                    "success": false,
                    "error": e.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&err_json).unwrap());
            } else {
                eprintln!("{} {}", "Error:".red().bold(), e);
            }
            std::process::exit(1);
        }
        Err(_) => {
            if output_format == "json" {
                let err_json = serde_json::json!({
                    "success": false,
                    "error": format!("Timed out after {}s", timeout_secs),
                });
                println!("{}", serde_json::to_string_pretty(&err_json).unwrap());
            } else {
                eprintln!(
                    "{} Timed out after {}s",
                    "Error:".red().bold(),
                    timeout_secs
                );
            }
            std::process::exit(1);
        }
    };

    match output_format {
        "json" => {
            let tool_calls: Vec<serde_json::Value> = response
                .tool_calls
                .iter()
                .map(|tc| {
                    serde_json::json!({
                        "name": tc.name,
                        "success": tc.success,
                        "duration_ms": tc.duration_ms,
                    })
                })
                .collect();

            let output = serde_json::json!({
                "success": true,
                "content": response.content,
                "iterations": response.iterations,
                "tool_calls": tool_calls,
                "tool_call_count": response.tool_calls.len(),
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            // Text output: just the content
            print!("{}", response.content);
            if !response.content.ends_with('\n') {
                println!();
            }

            if verbose {
                eprintln!(
                    "\n--- {} iterations, {} tool calls ---",
                    response.iterations,
                    response.tool_calls.len()
                );
                for tc in &response.tool_calls {
                    let s = if tc.success { "ok" } else { "FAIL" };
                    eprintln!("  [{}] {} ({}ms)", s, tc.name, tc.duration_ms);
                }
            }
        }
    }

    // Save session if requested
    if save_session {
        match agent.save_session() {
            Ok(id) => {
                if output_format == "json" {
                    // Already printed JSON above — add session id to stderr
                    eprintln!("Session saved: {}", id);
                } else {
                    eprintln!(
                        "Session saved: {} (resume with: pawan chat --resume {})",
                        id, id
                    );
                }
            }
            Err(e) => eprintln!("Warning: failed to save session: {}", e),
        }
    }

    Ok(())
}

/// Simple non-TUI interactive mode (fallback when TUI feature is disabled)
#[cfg(not(feature = "tui"))]
async fn run_simple_cli(mut agent: PawanAgent) -> Result<()> {
    use std::io::{BufRead, Write};

    println!("Pawan - Self-Healing CLI Coding Agent");
    println!("Type 'quit' or 'exit' to quit, 'clear' to clear history");
    println!("---");

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    loop {
        print!("> ");
        stdout.flush().ok();

        let mut line = String::new();
        stdin.lock().read_line(&mut line).ok();

        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if line == "quit" || line == "exit" {
            break;
        }

        if line == "clear" {
            agent.clear_history();
            println!("History cleared.");
            continue;
        }

        println!("\nProcessing...\n");

        match agent.execute(line).await {
            Ok(response) => {
                println!("{}\n", response.content);

                if !response.tool_calls.is_empty() {
                    println!("Tool calls made:");
                    for tc in &response.tool_calls {
                        let status = if tc.success { "✓" } else { "✗" };
                        println!("  {} {} ({}ms)", status, tc.name, tc.duration_ms);
                    }
                    println!();
                }
            }
            Err(e) => {
                println!("Error: {}\n", e);
            }
        }
    }

    Ok(())
}
