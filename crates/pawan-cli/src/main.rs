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

use clap::{CommandFactory, Parser, Subcommand};
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

    /// AI-powered commit: stage files, generate message, and commit
    #[command(alias = "ai-commit")]
    Commit {
        /// Stage all unstaged and untracked files before committing
        #[arg(short, long)]
        all: bool,

        /// Only generate the message, don't commit
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Improve the codebase
    Improve {
        /// What to improve: docs, refactor, tests, all
        target: String,

        /// Specific file or module to improve
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// Run tests and AI-analyze any failures
    Test {
        /// Specific test name or pattern to run
        #[arg(short, long)]
        filter: Option<String>,

        /// Auto-fix failing tests
        #[arg(long)]
        fix: bool,
    },

    /// AI-powered code review of current changes
    Review {
        /// Review staged changes only (default: all changes)
        #[arg(long)]
        staged: bool,

        /// Review a specific file
        #[arg(short, long)]
        file: Option<PathBuf>,
    },

    /// AI-powered explanation of a file, function, or concept
    Explain {
        /// What to explain: file path, function name, or concept
        query: String,
    },

    /// Show project status (errors, warnings, test failures)
    Status,

    /// Interactive chat mode (same as running without subcommand)
    Chat {
        /// Resume a saved session by ID
        #[arg(long)]
        resume: Option<String>,
    },

    /// Initialize pawan in a project (creates PAWAN.md + pawan.toml)
    Init,

    /// Diagnose setup issues (API keys, model connectivity, tools)
    Doctor,
    /// Run model latency benchmarks via nimakai
    Bench,
    /// Send a notification via doltares relay (WhatsApp/Telegram)
    Notify {
        /// Message to send
        message: String,

        /// Channel: whatsapp (default) or telegram
        #[arg(long, default_value = "whatsapp")]
        channel: String,
    },

    /// Format code with cargo fmt and cargo clippy --fix
    Fmt {
        /// Only check formatting without making changes
        #[arg(long)]
        check: bool,
    },

    /// Beads-style task tracking (deps, ready detection, memory decay)
    Tasks {
        #[command(subcommand)]
        action: TasksAction,
    },

    /// List saved sessions
    Sessions,

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

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

    /// Watch for errors and auto-heal (runs cargo check in a loop)
    Watch {
        /// Check interval in seconds
        #[arg(short, long, default_value = "10")]
        interval: u64,

        /// Auto-commit fixes
        #[arg(long)]
        commit: bool,

        /// Send notification via doltares relay on build failure
        #[arg(long)]
        notify: bool,
    },

    /// Distill a session into a reusable SKILL.md via thulpoff
    Distill {
        /// Session ID to distill (latest if omitted)
        #[arg(short, long)]
        session: Option<String>,

        /// Output directory for generated skill (default: ~/.pawan/skills/)
        #[arg(short, long)]
        output: Option<PathBuf>,
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

        /// Maximum retries for LLM API calls
        #[arg(long)]
        max_retries: Option<usize>,

        /// Save session after completion
        #[arg(long)]
        save: bool,

        /// Stream NDJSON events (requires --output json)
        #[arg(long)]
        stream: bool,
    },
}

#[derive(Subcommand)]
enum TasksAction {
    /// List beads (tasks) with optional filters
    List {
        /// Filter by status: open, in_progress, closed, all
        #[arg(long, default_value = "all")]
        status: String,
        /// Filter by max priority (0=critical, 4=backlog)
        #[arg(long)]
        priority: Option<u8>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show ready (actionable, unblocked) beads
    Ready {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a new bead
    Create {
        /// Title of the task
        title: String,
        /// Priority (0=critical, 4=backlog, default=2)
        #[arg(short, long, default_value = "2")]
        priority: u8,
        /// Description
        #[arg(short, long)]
        desc: Option<String>,
    },
    /// Update a bead
    Update {
        /// Bead ID (bd-XXXXXXXX or just XXXXXXXX)
        id: String,
        /// New status: open, in_progress, closed
        #[arg(long)]
        status: Option<String>,
        /// New priority
        #[arg(long)]
        priority: Option<u8>,
        /// New title
        #[arg(long)]
        title: Option<String>,
    },
    /// Close a bead
    Close {
        /// Bead ID
        id: String,
        /// Reason for closing
        #[arg(long)]
        reason: Option<String>,
    },
    /// Manage dependencies
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
    /// Run memory decay (archive old closed beads)
    Decay {
        /// Max age in days before archiving (default: 30)
        #[arg(long, default_value = "30")]
        max_age_days: u64,
    },
}

#[derive(Subcommand)]
enum DepAction {
    /// Add dependency: <id> depends on <blocks_id>
    Add {
        id: String,
        blocks_id: String,
    },
    /// Remove dependency
    Rm {
        id: String,
        blocks_id: String,
    },
}

#[derive(Subcommand)]
enum McpAction {
    /// List connected MCP servers and their tools
    List,
    /// Start pawan as an MCP server (stdio transport)
    Serve,
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
    // Auto-load .env file: try CWD first, then ~/.config/pawan/.env fallback
    // Always load config fallback to ensure NVIDIA_API_KEY is valid (CWD .env may have placeholder)
    dotenvy::dotenv().ok();
    if let Some(home) = dirs::home_dir() {
        let config_env = home.join(".config/pawan/.env");
        if config_env.exists() {
            // Override: config env takes precedence for pawan-specific keys
            dotenvy::from_path_override(&config_env).ok();
        }
    }

    let cli = Cli::parse();

    // Determine workspace root
    let workspace = cli
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Load configuration
    let mut config = PawanConfig::load(cli.config.as_ref())?;

    // Apply environment variable overrides (PAWAN_MODEL, PAWAN_PROVIDER, etc.)
    config.apply_env_overrides();

    // Apply CLI overrides (highest priority)
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
        Some(Commands::Init) => run_init(workspace).await,
        Some(Commands::Doctor) => run_doctor(config, workspace).await,
        Some(Commands::Bench) => run_bench().await,
        Some(Commands::Notify { message, channel }) => run_notify(&message, &channel).await,
        Some(Commands::Fmt { check }) => run_fmt(workspace, check).await,
        Some(Commands::Tasks { action }) => run_tasks(action).await,
        Some(Commands::Sessions) => run_sessions().await,
        Some(Commands::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "pawan", &mut std::io::stdout());
            Ok(())
        }
        Some(Commands::Mcp { action }) => match action {
            McpAction::List => run_mcp_list(config).await,
            McpAction::Serve => {
                pawan_mcp::server::serve(config).await?;
                Ok(())
            }
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
        Some(Commands::Commit { all, dry_run, yes }) => {
            run_commit(config, workspace, all, dry_run, yes).await
        }
        Some(Commands::Improve { target, file }) => {
            run_improve(config, workspace, &target, file, cli.verbose).await
        }
        Some(Commands::Test { filter, fix }) => run_test(config, workspace, filter, fix).await,
        Some(Commands::Review { staged, file }) => {
            run_review(config, workspace, staged, file).await
        }
        Some(Commands::Explain { query }) => run_explain(config, workspace, &query).await,
        Some(Commands::Distill { session, output }) => {
            run_distill(config, session, output).await
        }
        Some(Commands::Status) => run_status(config, workspace).await,
        Some(Commands::Watch { interval, commit, notify }) => {
            run_watch(config, workspace, interval, commit, notify).await
        }
        Some(Commands::Run {
            prompt,
            file,
            output,
            timeout,
            max_iterations,
            max_retries,
            save,
            stream,
        }) => {
            run_headless(
                config,
                workspace,
                prompt,
                file,
                &output,
                timeout,
                max_iterations,
                max_retries,
                save,
                stream,
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
async fn run_commit(
    config: PawanConfig,
    workspace: PathBuf,
    stage_all: bool,
    dry_run: bool,
    auto_yes: bool,
) -> Result<()> {
    use dialoguer::{Confirm, MultiSelect};

    // 1. Show current git status
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&workspace)
        .output()
        .map_err(PawanError::Io)?;
    let status_text = String::from_utf8_lossy(&status_output.stdout);

    if status_text.trim().is_empty() {
        println!("{}", "Nothing to commit — working tree clean.".dimmed());
        return Ok(());
    }

    // Parse files into categories
    let mut staged: Vec<String> = Vec::new();
    let mut unstaged: Vec<String> = Vec::new();
    let mut untracked: Vec<String> = Vec::new();

    for line in status_text.lines() {
        if line.len() < 4 {
            continue;
        }
        let index_status = line.chars().next().unwrap_or(' ');
        let worktree_status = line.chars().nth(1).unwrap_or(' ');
        let file = line[3..].trim().to_string();

        if index_status == '?' {
            untracked.push(file);
        } else {
            if index_status != ' ' && index_status != '?' {
                staged.push(file.clone());
            }
            if worktree_status != ' ' && worktree_status != '?' {
                unstaged.push(file);
            }
        }
    }

    // Display status summary
    if !staged.is_empty() {
        println!("{}", "Staged:".green().bold());
        for f in &staged {
            println!("  {} {}", "✓".green(), f);
        }
    }
    if !unstaged.is_empty() {
        println!("{}", "Unstaged:".yellow().bold());
        for f in &unstaged {
            println!("  {} {}", "~".yellow(), f);
        }
    }
    if !untracked.is_empty() {
        println!("{}", "Untracked:".red().bold());
        for f in &untracked {
            println!("  {} {}", "?".red(), f);
        }
    }
    println!();

    // 2. Stage files
    let needs_staging = !unstaged.is_empty() || !untracked.is_empty();

    if needs_staging {
        if stage_all {
            // Stage everything
            println!("{}", "Staging all files...".cyan());
            let output = std::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(&workspace)
                .output()
                .map_err(PawanError::Io)?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(PawanError::Git(format!("git add -A failed: {}", stderr)));
            }
        } else if staged.is_empty() {
            // Nothing staged yet — prompt user to select files
            let mut all_files: Vec<String> = Vec::new();
            all_files.extend(unstaged.iter().map(|f| format!("~ {}", f)));
            all_files.extend(untracked.iter().map(|f| format!("? {}", f)));

            let selections = MultiSelect::new()
                .with_prompt("Select files to stage (space to toggle, enter to confirm)")
                .items(&all_files)
                .defaults(&vec![true; all_files.len()])
                .interact()
                .unwrap_or_default();

            if selections.is_empty() {
                println!("{}", "No files selected. Aborting.".dimmed());
                return Ok(());
            }

            let mut files_to_add: Vec<String> = Vec::new();
            for idx in selections {
                let raw = &all_files[idx];
                // Strip the "~ " or "? " prefix
                files_to_add.push(raw[2..].to_string());
            }

            let file_refs: Vec<&str> = files_to_add.iter().map(|s| s.as_str()).collect();
            let mut args = vec!["add", "--"];
            args.extend(file_refs);

            let output = std::process::Command::new("git")
                .args(&args)
                .current_dir(&workspace)
                .output()
                .map_err(PawanError::Io)?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(PawanError::Git(format!("git add failed: {}", stderr)));
            }

            println!(
                "{}",
                format!("Staged {} file(s).", files_to_add.len()).green()
            );
        }
    }

    // 3. Generate AI commit message from staged diff
    println!("{}", "Generating commit message...".cyan());

    let diff_output = std::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(&workspace)
        .output()
        .map_err(PawanError::Io)?;
    let diff_stat = String::from_utf8_lossy(&diff_output.stdout);

    let diff_output = std::process::Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(&workspace)
        .output()
        .map_err(PawanError::Io)?;
    let diff_full = String::from_utf8_lossy(&diff_output.stdout);

    if diff_full.trim().is_empty() && staged.is_empty() {
        println!(
            "{}",
            "No staged changes to commit. Use -a to stage all.".dimmed()
        );
        return Ok(());
    }

    // Truncate diff if too long to avoid token waste
    let diff_for_prompt = if diff_full.len() > 8000 {
        format!("{}...\n\n[diff truncated, {} total bytes]", &diff_full[..8000], diff_full.len())
    } else {
        diff_full.to_string()
    };

    let prompt = format!(
        r#"Generate a concise git commit message for the following changes.

Rules:
- Use conventional commits format (feat:, fix:, refactor:, chore:, docs:, test:)
- First line under 72 chars
- Add a blank line then a brief body (2-4 bullet points) if the changes are non-trivial
- Output ONLY the commit message, nothing else — no markdown fences, no explanation

Diff stat:
{diff_stat}

Full diff:
{diff_for_prompt}"#
    );

    let mut agent = PawanAgent::new(config, workspace.clone());
    let response = agent.execute(&prompt).await?;
    let message = response.content.trim().to_string();

    // Strip markdown code fences if the model wraps the output
    let message = message
        .strip_prefix("```")
        .unwrap_or(&message)
        .strip_suffix("```")
        .unwrap_or(&message)
        .trim()
        .to_string();

    // Show diff preview
    println!("\n{}", "Diff preview:".cyan().bold());
    println!("{}", "─".repeat(50).dimmed());
    println!("{}", diff_stat.trim());
    println!("{}", "─".repeat(50).dimmed());
    let diff_lines: Vec<&str> = diff_full.lines().take(40).collect();
    for line in &diff_lines {
        println!("{}", line);
    }
    if diff_full.lines().count() > 40 {
        println!("{}", format!("... [{} more lines]", diff_full.lines().count() - 40).dimmed());
    }
    println!("{}", "─".repeat(50).dimmed());
    println!("\n{}", "Commit message:".green().bold());
    println!("{}", "─".repeat(50).dimmed());
    println!("{}", message);
    println!("{}", "─".repeat(50).dimmed());

    if dry_run {
        println!("\n{}", "(dry run — not committing)".dimmed());
        return Ok(());
    }

    // 4. Confirm and commit
    let should_commit = auto_yes
        || Confirm::new()
            .with_prompt("Commit with this message?")
            .default(true)
            .interact()
            .unwrap_or(false);

    if should_commit {
        let output = std::process::Command::new("git")
            .args(["commit", "-m", &message])
            .current_dir(&workspace)
            .output()
            .map_err(PawanError::Io)?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!("{} {}", "✓".green(), "Committed!".green().bold());
            // Extract and show commit hash
            if let Some(line) = stdout.lines().next() {
                println!("  {}", line.dimmed());
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(PawanError::Git(format!("git commit failed: {}", stderr)));
        }
    } else {
        println!("{}", "Aborted.".dimmed());
    }

    Ok(())
}

/// AI-powered test runner and failure analysis
async fn run_test(
    config: PawanConfig,
    workspace: PathBuf,
    filter: Option<String>,
    auto_fix: bool,
) -> Result<()> {
    // Run cargo test
    let mut test_args = vec!["test", "--workspace"];
    let filter_owned;
    if let Some(ref f) = filter {
        filter_owned = f.clone();
        test_args.push("--");
        test_args.push(&filter_owned);
    }

    println!(
        "{} {}",
        "Running".cyan(),
        if let Some(ref f) = filter {
            format!("tests matching '{}'...", f)
        } else {
            "all tests...".to_string()
        }
    );

    let output = std::process::Command::new("cargo")
        .args(&test_args)
        .current_dir(&workspace)
        .output()
        .map_err(PawanError::Io)?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    // Count results
    let passed = combined.matches("test result: ok.").count();
    let has_failures = combined.contains("FAILED") || combined.contains("failures:");

    if !has_failures {
        println!(
            "{} {} {}",
            "✓".green(),
            "All tests passed!".green().bold(),
            format!("({} suite(s))", passed).dimmed()
        );
        return Ok(());
    }

    // Extract failure info
    let failure_lines: Vec<&str> = combined
        .lines()
        .filter(|l| {
            l.contains("FAILED")
                || l.contains("panicked at")
                || l.contains("assertion")
                || l.contains("failures:")
                || l.contains("---- ")
        })
        .collect();

    println!(
        "\n{} {}",
        "✗".red(),
        "Test failures detected:".red().bold()
    );
    for line in &failure_lines {
        println!("  {}", line);
    }

    if !auto_fix {
        println!(
            "\n{}",
            "Run with --fix to auto-fix failures.".dimmed()
        );
        return Ok(());
    }

    // AI-powered fix
    println!("\n{}", "Analyzing and fixing failures...".cyan());

    let test_output = if combined.len() > 8000 {
        format!("{}...\n[truncated, {} bytes total]", &combined[..8000], combined.len())
    } else {
        combined.to_string()
    };

    let prompt = format!(
        r#"The following test failures occurred in the project at {}:

```
{}
```

Please:
1. Analyze each failure to understand the root cause
2. Read the relevant source and test files
3. Fix each failure — prefer fixing the implementation over the test unless the test is clearly wrong
4. Run `cargo test` to verify your fixes work"#,
        workspace.display(),
        test_output
    );

    let mut agent = PawanAgent::new(config, workspace);

    let on_token: pawan::agent::TokenCallback = Box::new(|token: &str| {
        use std::io::Write;
        print!("{}", token);
        std::io::stdout().flush().ok();
    });

    let response = agent
        .execute_with_callbacks(&prompt, Some(on_token), None, None)
        .await?;

    if !response.content.ends_with('\n') {
        println!();
    }

    println!(
        "\n{} {}",
        "Done.".green(),
        format!("({} iterations, {} tool calls)", response.iterations, response.tool_calls.len()).dimmed()
    );

    Ok(())
}

/// AI-powered explanation
async fn run_explain(config: PawanConfig, workspace: PathBuf, query: &str) -> Result<()> {
    println!("{} {}", "Explaining:".cyan(), query);

    let prompt = if std::path::Path::new(query).exists() || query.contains('/') || query.contains('.') {
        format!(
            r#"Read the file at `{query}` and explain it concisely:
1. What it does (purpose)
2. Key types/functions and their roles
3. How it fits into the broader codebase
4. Any notable patterns, dependencies, or gotchas

Be concise — aim for 10-20 lines. Skip obvious things."#
        )
    } else {
        format!(
            "In the context of this codebase at {}, explain: {}\n\n\
             If this is a function/type name, find it in the code first.\n\
             Be concise — aim for 10-20 lines.",
            workspace.display(),
            query
        )
    };

    let mut agent = PawanAgent::new(config, workspace);

    let on_token: pawan::agent::TokenCallback = Box::new(|token: &str| {
        use std::io::Write;
        print!("{}", token);
        std::io::stdout().flush().ok();
    });

    let response = agent
        .execute_with_callbacks(&prompt, Some(on_token), None, None)
        .await?;

    if !response.content.ends_with('\n') {
        println!();
    }

    Ok(())
}

/// AI-powered code review
async fn run_review(
    config: PawanConfig,
    workspace: PathBuf,
    staged_only: bool,
    file: Option<PathBuf>,
) -> Result<()> {
    // Get the diff
    let mut diff_args: Vec<String> = if staged_only {
        vec!["diff".into(), "--cached".into()]
    } else {
        vec!["diff".into(), "HEAD".into()]
    };

    if let Some(ref f) = file {
        diff_args.push("--".into());
        diff_args.push(f.to_string_lossy().into_owned());
    }

    let diff_args_ref: Vec<&str> = diff_args.iter().map(|s| s.as_str()).collect();
    let diff_output = std::process::Command::new("git")
        .args(&diff_args_ref)
        .current_dir(&workspace)
        .output()
        .map_err(PawanError::Io)?;

    let diff = String::from_utf8_lossy(&diff_output.stdout);

    if diff.trim().is_empty() {
        // Try unstaged diff if HEAD diff is empty
        let fallback = std::process::Command::new("git")
            .args(["diff"])
            .current_dir(&workspace)
            .output()
            .map_err(PawanError::Io)?;
        let fallback_diff = String::from_utf8_lossy(&fallback.stdout);

        if fallback_diff.trim().is_empty() {
            println!("{}", "No changes to review.".dimmed());
            return Ok(());
        }
        // Use unstaged diff
        return run_review_with_diff(config, workspace, &fallback_diff).await;
    }

    run_review_with_diff(config, workspace, &diff).await
}

async fn run_review_with_diff(
    config: PawanConfig,
    workspace: PathBuf,
    diff: &str,
) -> Result<()> {
    println!(
        "{} {}",
        "Reviewing".cyan(),
        format!("({} lines of diff)...", diff.lines().count()).dimmed()
    );

    let diff_text = if diff.len() > 12000 {
        format!(
            "{}...\n\n[diff truncated, {} total bytes]",
            &diff[..12000],
            diff.len()
        )
    } else {
        diff.to_string()
    };

    let prompt = format!(
        r#"Review the following code changes. Be concise and actionable.

For each issue found, output:
- **Severity**: 🔴 critical / 🟡 warning / 🔵 suggestion
- **Location**: file and line
- **Issue**: what's wrong
- **Fix**: how to fix it

At the end, give an overall assessment: LGTM ✅, needs fixes 🔧, or needs rework ❌.

Focus on: bugs, security issues, performance, error handling, edge cases, code style.
Do NOT nitpick formatting or suggest adding comments.

```diff
{diff_text}
```"#
    );

    let mut agent = PawanAgent::new(config, workspace);

    // Stream the review output
    let on_token: pawan::agent::TokenCallback = Box::new(|token: &str| {
        use std::io::Write;
        print!("{}", token);
        std::io::stdout().flush().ok();
    });

    let response = agent
        .execute_with_callbacks(&prompt, Some(on_token), None, None)
        .await?;

    // Ensure final newline
    if !response.content.ends_with('\n') {
        println!();
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
/// Watch mode: poll cargo check and auto-heal on errors
async fn run_watch(
    config: PawanConfig,
    workspace: PathBuf,
    interval_secs: u64,
    auto_commit: bool,
    notify: bool,
) -> Result<()> {
    use std::io::Write;

    println!(
        "{}",
        format!(
            "Watching {} every {}s (Ctrl+C to stop)",
            workspace.display(),
            interval_secs
        )
        .cyan()
    );

    let mut last_status = true; // assume healthy at start
    let mut heal_count = 0u32;

    loop {
        // Run cargo check
        let check = std::process::Command::new("cargo")
            .args(["check", "--workspace", "--message-format=short"])
            .current_dir(&workspace)
            .output()
            .map_err(PawanError::Io)?;

        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let secs = elapsed.as_secs() % 86400;
        let now = format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60);

        if check.status.success() {
            if !last_status {
                println!("{} {} {}", format!("[{}]", now).dimmed(), "✓".green(), "All clear — project compiles.".green());
            } else {
                print!("{} {} {}\r", format!("[{}]", now).dimmed(), "✓".green(), "OK".dimmed());
                std::io::stdout().flush().ok();
            }
            last_status = true;
        } else {
            let stderr = String::from_utf8_lossy(&check.stderr);
            let error_count = stderr.lines().filter(|l| l.contains("error[")).count();
            let warning_count = stderr.lines().filter(|l| l.contains("warning:")).count();

            println!(
                "\n{} {} {}",
                format!("[{}]", now).dimmed(),
                "✗".red(),
                format!("{} error(s), {} warning(s) — healing...", error_count, warning_count).red()
            );

            last_status = false;
            heal_count += 1;

            // Send notification on build failure if --notify
            if notify {
                let _ = run_notify(
                    &format!("[pawan-watch] Build failed: {} error(s), {} warning(s) in {}",
                        error_count, warning_count, workspace.display()),
                    "whatsapp",
                ).await;
            }

            // Auto-heal
            let mut agent = PawanAgent::new(config.clone(), workspace.clone());

            let on_token: pawan::agent::TokenCallback = Box::new(|token: &str| {
                use std::io::Write;
                print!("{}", token);
                std::io::stdout().flush().ok();
            });

            match agent
                .execute_with_callbacks(
                    &format!(
                        "Fix the compilation errors in this project at {}. \
                         Run `cargo check` to see errors, then fix them one at a time. \
                         Verify each fix compiles before moving on.",
                        workspace.display()
                    ),
                    Some(on_token),
                    None,
                    None,
                )
                .await
            {
                Ok(resp) => {
                    println!("\n{}", format!("Heal #{} complete ({} iterations, {} tool calls)", heal_count, resp.iterations, resp.tool_calls.len()).green());

                    if auto_commit && !resp.tool_calls.is_empty() {
                        let commit_output = std::process::Command::new("git")
                            .args(["add", "-A"])
                            .current_dir(&workspace)
                            .output()
                            .ok();

                        if commit_output.map(|o| o.status.success()).unwrap_or(false) {
                            let msg = format!("fix: auto-heal #{} by pawan watch", heal_count);
                            let _ = std::process::Command::new("git")
                                .args(["commit", "-m", &msg])
                                .current_dir(&workspace)
                                .output();
                            println!("{}", format!("  Auto-committed: {}", msg).dimmed());
                        }
                    }
                }
                Err(e) => {
                    println!("\n{}", format!("Heal failed: {}", e).red());
                }
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
    }
}

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
/// Diagnose setup issues
async fn run_doctor(config: PawanConfig, workspace: PathBuf) -> Result<()> {
    use pawan::config::LlmProvider;

    println!("{}", "Pawan Doctor".cyan().bold());
    println!("{}\n", "─".repeat(40).dimmed());

    let mut issues = 0u32;

    // 1. Check workspace
    print!("  Workspace: ");
    if workspace.exists() {
        println!("{} {}", "✓".green(), workspace.display());
    } else {
        println!("{} {} (not found)", "✗".red(), workspace.display());
        issues += 1;
    }

    // 2. Check config files
    print!("  pawan.toml: ");
    if workspace.join("pawan.toml").exists() {
        println!("{}", "✓ found".green());
    } else {
        println!("{}", "- not found (using defaults)".dimmed());
    }

    print!("  PAWAN.md: ");
    if workspace.join("PAWAN.md").exists() {
        println!("{}", "✓ found".green());
    } else {
        println!("{}", "- not found (run `pawan init`)".dimmed());
    }

    // 3. Check .env
    print!("  .env: ");
    if workspace.join(".env").exists() || std::path::Path::new(".env").exists() {
        println!("{}", "✓ found".green());
    } else {
        println!("{}", "- not found".dimmed());
    }

    // 4. Check API keys
    println!("\n{}", "  API Keys:".bold());
    match config.provider {
        LlmProvider::Nvidia => {
            print!("    NVIDIA_API_KEY: ");
            if std::env::var("NVIDIA_API_KEY").is_ok() {
                println!("{}", "✓ set".green());
            } else {
                println!("{}", "✗ NOT SET".red());
                issues += 1;
            }
        }
        LlmProvider::OpenAI => {
            print!("    OPENAI_API_KEY: ");
            if std::env::var("OPENAI_API_KEY").is_ok() {
                println!("{}", "✓ set".green());
            } else {
                println!("{}", "✗ NOT SET".red());
                issues += 1;
            }
        }
        LlmProvider::Ollama => {
            print!("    Ollama URL: ");
            let url = std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
            println!("{}", url.cyan());
        }
        LlmProvider::Mlx => {
            print!("    MLX URL: ");
            let url = std::env::var("MLX_URL").unwrap_or_else(|_| "http://localhost:8080".into());
            println!("{}", url.cyan());
        }
    }

    // 5. Check model connectivity
    println!("\n{}", "  Model:".bold());
    println!("    Provider: {}", format!("{:?}", config.provider).cyan());
    println!("    Model: {}", config.model.cyan());

    print!("    Connectivity: ");
    // Quick ping test
    let api_url = match config.provider {
        LlmProvider::Nvidia => {
            std::env::var("NVIDIA_API_URL")
                .unwrap_or_else(|_| pawan::DEFAULT_NVIDIA_API_URL.to_string())
        }
        LlmProvider::OpenAI => {
            std::env::var("OPENAI_API_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string())
        }
        LlmProvider::Ollama => {
            std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string())
        }
        LlmProvider::Mlx => {
            std::env::var("MLX_URL")
                .unwrap_or_else(|_| "http://localhost:8080".to_string())
        }
    };

    let ping_url = if matches!(config.provider, LlmProvider::Ollama) {
        api_url.clone()
    } else {
        format!("{}/models", api_url)
    };

    match std::process::Command::new("curl")
        .args(["-sS", "--max-time", "5", "-o", "/dev/null", "-w", "%{http_code}", &ping_url])
        .output()
    {
        Ok(output) if output.status.success() => {
            let code = String::from_utf8_lossy(&output.stdout);
            let code = code.trim();
            if code == "200" || code == "401" {
                println!("{} (reachable)", "✓".green());
            } else {
                println!("{} (HTTP {})", "⚠".yellow(), code);
            }
        }
        Ok(_) => {
            println!("{}", "✗ unreachable".red());
            issues += 1;
        }
        Err(_) => {
            println!("{}", "✗ curl not found".red());
            issues += 1;
        }
    }

    // 6. Check git
    println!("\n{}", "  Git:".bold());
    print!("    Repository: ");
    let git_check = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&workspace)
        .output();
    match git_check {
        Ok(output) if output.status.success() => println!("{}", "✓ inside git repo".green()),
        _ => println!("{}", "- not a git repo".dimmed()),
    }

    // 7. Check tools
    println!("\n{}", "  Tools:".bold());
    let agent = PawanAgent::new(config.clone(), workspace);
    let tool_count = agent.get_tool_definitions().len();
    println!("    Registered: {} tools", format!("{}", tool_count).cyan());

    // 8. MCP servers
    if !config.mcp.is_empty() {
        println!("\n{}", "  MCP Servers:".bold());
        for (name, entry) in &config.mcp {
            let status = if entry.enabled { "enabled" } else { "disabled" };
            println!(
                "    {}: {} ({})",
                name,
                entry.command.cyan(),
                if entry.enabled {
                    status.green().to_string()
                } else {
                    status.dimmed().to_string()
                }
            );
        }
    }

    // 9. Check native tool binaries (inspired by gstack health checks)
    println!("\n{}", "  Native Binaries:".bold());
    let binaries = [
        ("rg", "ripgrep — fast code search"),
        ("fd", "fd — fast file finder"),
        ("ast-grep", "ast-grep — structural search/replace"),
        ("bat", "bat — syntax-highlighted file viewer"),
        ("delta", "delta — git diff viewer"),
    ];
    let mut missing_count = 0;
    for (bin, desc) in &binaries {
        if which::which(bin).is_ok() {
            println!("    {} {} {}", "✓".green(), bin, format!("({})", desc).dimmed());
        } else {
            println!("    {} {} {}", "-".dimmed(), bin, format!("({})", desc).dimmed());
            missing_count += 1;
        }
    }
    if missing_count > 0 {
        println!("    {}", "Install missing: mise install <name>".dimmed());
    }

    // Summary
    println!("\n{}", "─".repeat(40).dimmed());
    if issues == 0 {
        println!("{}", "  All checks passed! ✓".green().bold());
    } else {
        println!(
            "{}",
            format!("  {} issue(s) found.", issues).yellow().bold()
        );
    }

    Ok(())
}

/// Initialize pawan in a project
async fn run_init(workspace: PathBuf) -> Result<()> {
    let mut created = Vec::new();

    // Create pawan.toml if not exists
    let toml_path = workspace.join("pawan.toml");
    if !toml_path.exists() {
        let toml_content = r#"# Pawan configuration
# See: https://github.com/dirmacs/pawan

# LLM provider: nvidia, ollama, openai
provider = "nvidia"

# Model to use
model = "nvidia/llama-3.3-nemotron-super-49b-v1"

# Temperature (0.0 - 2.0)
temperature = 1.0

# Maximum tokens in response
max_tokens = 8192

# Maximum tool iterations per request
max_tool_iterations = 15

# [mcp.daedra]
# command = "daedra"
# args = ["serve", "--transport", "stdio", "--quiet"]
"#;
        std::fs::write(&toml_path, toml_content).map_err(PawanError::Io)?;
        created.push("pawan.toml");
    }

    // Create PAWAN.md if not exists
    let md_path = workspace.join("PAWAN.md");
    if !md_path.exists() {
        // Try to detect project info from Cargo.toml or package.json
        let project_name = if let Ok(cargo) =
            std::fs::read_to_string(workspace.join("Cargo.toml"))
        {
            cargo
                .lines()
                .find(|l| l.starts_with("name"))
                .and_then(|l| l.split('"').nth(1))
                .unwrap_or("this project")
                .to_string()
        } else {
            workspace
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "this project".to_string())
        };

        let md_content = format!(
            r#"# {project_name}

## Project Context

<!-- Pawan reads this file to understand your project. -->
<!-- Add project-specific instructions, conventions, and context here. -->

## Architecture

<!-- Describe the high-level architecture -->

## Conventions

<!-- Code style, naming conventions, patterns to follow -->

## Key Files

<!-- List important files and what they do -->
"#
        );
        std::fs::write(&md_path, md_content).map_err(PawanError::Io)?;
        created.push("PAWAN.md");
    }

    // Create .pawan/ directory
    let pawan_dir = workspace.join(".pawan");
    if !pawan_dir.exists() {
        std::fs::create_dir_all(&pawan_dir).map_err(PawanError::Io)?;
        created.push(".pawan/");
    }

    if created.is_empty() {
        println!(
            "{}",
            "Pawan is already initialized in this directory.".dimmed()
        );
    } else {
        println!("{}", "Pawan initialized!".green().bold());
        for f in &created {
            println!("  {} {}", "✓".green(), f);
        }
        println!(
            "\n{}",
            "Edit PAWAN.md to add project context for better AI assistance.".dimmed()
        );
    }

    Ok(())
}

async fn run_tasks(action: TasksAction) -> Result<()> {
    use pawan::tasks::{BeadId, BeadStatus, BeadStore};

    let store = BeadStore::open()?;

    match action {
        TasksAction::List { status, priority, json } => {
            let status_filter = match status.as_str() {
                "all" => None,
                s => Some(s),
            };
            let beads = store.list(status_filter, priority)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&beads).unwrap_or_default());
                return Ok(());
            }

            if beads.is_empty() {
                println!("{}", "No beads found.".dimmed());
                return Ok(());
            }

            println!("{}", "Beads".green().bold());
            println!("{}", "═".repeat(70).dimmed());
            println!(
                "  {:<12} {:<5} {:<12} {}",
                "ID".cyan(), "Pri".cyan(), "Status".cyan(), "Title".cyan()
            );
            println!("{}", "─".repeat(70).dimmed());

            for b in &beads {
                let status_color = match b.status {
                    BeadStatus::Open => format!("{:<12}", "open").yellow().to_string(),
                    BeadStatus::InProgress => format!("{:<12}", "in_progress").blue().to_string(),
                    BeadStatus::Closed => format!("{:<12}", "closed").dimmed().to_string(),
                };
                println!("  {:<12} {:<5} {} {}", b.id.display(), b.priority, status_color, b.title);
            }
            println!("\n  {} beads total", beads.len());
        }

        TasksAction::Ready { json } => {
            let beads = store.ready()?;

            if json {
                println!("{}", serde_json::to_string_pretty(&beads).unwrap_or_default());
                return Ok(());
            }

            if beads.is_empty() {
                println!("{}", "No ready beads (all blocked or none open).".dimmed());
                return Ok(());
            }

            println!("{} {} actionable beads:", "Ready:".green().bold(), beads.len());
            for b in &beads {
                println!("  {} [P{}] {}", b.id.display().cyan(), b.priority, b.title);
            }
        }

        TasksAction::Create { title, priority, desc } => {
            let bead = store.create(&title, desc.as_deref(), priority)?;
            println!("{} {} — {}", "Created:".green().bold(), bead.id.display().cyan(), bead.title);
        }

        TasksAction::Update { id, status, priority, title } => {
            let bid = BeadId::parse(&id);
            let status = status.map(|s| s.parse::<BeadStatus>().unwrap_or(BeadStatus::Open));
            store.update(&bid, title.as_deref(), status, priority)?;
            println!("{} {}", "Updated:".green().bold(), bid.display().cyan());
        }

        TasksAction::Close { id, reason } => {
            let bid = BeadId::parse(&id);
            store.close(&bid, reason.as_deref())?;
            println!("{} {}", "Closed:".green().bold(), bid.display().cyan());
        }

        TasksAction::Dep { action } => match action {
            DepAction::Add { id, blocks_id } => {
                let bid = BeadId::parse(&id);
                let dep = BeadId::parse(&blocks_id);
                store.dep_add(&bid, &dep)?;
                println!("{} {} depends on {}", "Dep added:".green().bold(), bid.display().cyan(), dep.display().cyan());
            }
            DepAction::Rm { id, blocks_id } => {
                let bid = BeadId::parse(&id);
                let dep = BeadId::parse(&blocks_id);
                store.dep_remove(&bid, &dep)?;
                println!("{} {} no longer depends on {}", "Dep removed:".green().bold(), bid.display().cyan(), dep.display().cyan());
            }
        },

        TasksAction::Decay { max_age_days } => {
            let count = store.memory_decay(max_age_days)?;
            if count == 0 {
                println!("{}", "No beads to archive.".dimmed());
            } else {
                println!("{} {} beads archived (older than {} days)", "Decayed:".green().bold(), count, max_age_days);
            }
        }
    }

    Ok(())
}

async fn run_distill(
    config: PawanConfig,
    session_id: Option<String>,
    output_dir: Option<PathBuf>,
) -> Result<()> {
    use pawan::agent::session::Session;
    use pawan::agent::TokenUsage;
    use pawan::skill_distillation;

    // Resolve which session to distill
    let session = match session_id {
        Some(id) => Session::load(&id)?,
        None => {
            let sessions = Session::list()?;
            let latest = sessions.first().ok_or_else(|| {
                PawanError::NotFound("No saved sessions found. Run a task first, then distill.".into())
            })?;
            eprintln!("Using latest session: {}", latest.id);
            Session::load(&latest.id)?
        }
    };

    // Check if session is worth distilling
    if !skill_distillation::is_distillable(&session) {
        eprintln!(
            "Session {} has insufficient content for distillation (needs tool calls + messages).",
            session.id
        );
        return Ok(());
    }

    let usage = TokenUsage {
        prompt_tokens: session.total_tokens / 2,
        completion_tokens: session.total_tokens / 2,
        total_tokens: session.total_tokens,
        ..Default::default()
    };

    let output = output_dir
        .or_else(|| skill_distillation::skills_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    eprintln!(
        "Distilling session {} ({} messages, {} tool calls)...",
        session.id,
        session.messages.len(),
        session.messages.iter().flat_map(|m| m.tool_calls.iter()).count()
    );

    match skill_distillation::distill_and_save(&session, &usage, &config, &output).await {
        Ok(path) => {
            println!("Skill distilled to: {}", path.display());
            Ok(())
        }
        Err(e) => {
            eprintln!("Distillation failed: {}", e);
            Err(e)
        }
    }
}

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
    use owo_colors::OwoColorize;
    println!("{}", "Pawan Configuration (resolved)".cyan().bold());
    println!("{}\n", "─".repeat(40).dimmed());

    println!("  {} {}", "Provider:".bold(), format!("{:?}", config.provider).cyan());
    println!("  {} {}", "Model:".bold(), config.model.cyan());
    println!("  {} {}", "Temperature:".bold(), config.temperature);
    println!("  {} {}", "Max tokens:".bold(), config.max_tokens);
    println!("  {} {}", "Max iterations:".bold(), config.max_tool_iterations);
    println!("  {} {}", "Thinking mode:".bold(), if config.use_thinking_mode() { "enabled".green().to_string() } else { "disabled".dimmed().to_string() });

    if let Some(ref cloud) = config.cloud {
        println!("\n{}", "  Cloud fallback:".bold());
        println!("    Model: {}", cloud.model.cyan());
    }

    if !config.fallback_models.is_empty() {
        println!("  {} {}", "Fallbacks:".bold(), config.fallback_models.join(", "));
    }

    println!("\n{}", "  Healing:".bold());
    println!("    Errors: {}", if config.healing.fix_errors { "fix" } else { "skip" });
    println!("    Warnings: {}", if config.healing.fix_warnings { "fix" } else { "skip" });
    println!("    Tests: {}", if config.healing.fix_tests { "fix" } else { "skip" });

    if !config.permissions.is_empty() {
        println!("\n{}", "  Permissions:".bold());
        for (tool, perm) in &config.permissions {
            println!("    {}: {:?}", tool, perm);
        }
    }

    if !config.mcp.is_empty() {
        println!("\n{}", "  MCP servers:".bold());
        for (name, entry) in &config.mcp {
            let status = if entry.enabled { "enabled".green().to_string() } else { "disabled".dimmed().to_string() };
            println!("    {}: {} ({})", name, entry.command.cyan(), status);
        }
    }

    println!("\n{}", "  Context files:".bold());
    for path in &["PAWAN.md", "AGENTS.md", "CLAUDE.md", "SKILL.md", ".pawan/context.md"] {
        if std::path::Path::new(path).exists() {
            println!("    {} {}", "✓".green(), path);
        }
    }

    println!("\n{}", "─".repeat(40).dimmed());
    println!("{}", "  Use `pawan config init` to generate pawan.toml".dimmed());
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
model = "qwen/qwen3.5-122b-a10b"

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
    max_retries: Option<usize>,
    save_session: bool,
    stream: bool,
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
    if let Some(retries) = max_retries {
        config.max_retries = retries;
    }

    let config_ref = config.clone();
    let mut agent = PawanAgent::new(config, workspace);

    #[cfg(feature = "mcp")]
    setup_mcp_tools(&mut agent, &config_ref).await;

    // Pre-flight: verify model is reachable before starting work
    if let Err(e) = agent.preflight_check().await {
        if output_format == "json" {
            println!("{}", serde_json::json!({"error": e.to_string(), "success": false}));
        } else {
            eprintln!("\x1b[31mModel health check failed:\x1b[0m {}", e);
            eprintln!("Check: is the model server running? Is the API URL correct?");
        }
        std::process::exit(1);
    }

    let is_json = output_format == "json";
    let use_color = !is_json && atty::is(atty::Stream::Stderr);

    // Header
    if !is_json {
        if use_color {
            eprintln!("\x1b[1;36m┌─ pawan run\x1b[0m");
            eprintln!("\x1b[1;36m│\x1b[0m \x1b[33mModel:\x1b[0m  {}", agent.config().model);
            let display_prompt: String = prompt_text.chars().take(80).collect();
            eprintln!("\x1b[1;36m│\x1b[0m \x1b[33mPrompt:\x1b[0m {}", display_prompt);
            eprintln!("\x1b[1;36m└─\x1b[0m");
        } else {
            eprintln!("── pawan run ──");
            eprintln!("Model:  {}", agent.config().model);
            let display_prompt: String = prompt_text.chars().take(80).collect();
            eprintln!("Prompt: {}", display_prompt);
            eprintln!("───────────────");
        }
    }

    // Token streaming callback — streams content to stdout, strips thinking
    let use_color_token = use_color;
    let on_token: Option<pawan::agent::TokenCallback> = if stream && is_json {
        Some(Box::new(|token: &str| {
            use std::io::Write;
            let event = serde_json::json!({"type": "token", "content": token});
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
            std::io::stdout().flush().ok();
        }))
    } else if !is_json {
        // Stateful token filter: suppresses [TOOL_CALLS], <think>, and model narration
        let suppressing = std::sync::Arc::new(std::sync::Mutex::new(false));
        let buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        Some(Box::new(move |token: &str| {
            use std::io::Write;
            let mut sup = suppressing.lock().unwrap_or_else(|e| e.into_inner());
            let mut buf = buffer.lock().unwrap_or_else(|e| e.into_inner());

            // Accumulate into buffer for pattern detection
            buf.push_str(token);

            // Suppress [TOOL_CALLS] blocks — model emits these as text before actual tool call
            if buf.contains("[TOOL_CALLS]") || buf.contains("[TOOL_CALL]") {
                *sup = true;
                buf.clear();
                return;
            }

            // If suppressing, eat tokens until newline (tool call JSON ends at newline)
            if *sup {
                if token.contains('\n') {
                    *sup = false;
                    buf.clear();
                }
                return;
            }

            // Strip thinking tags
            let clean = buf
                .replace("<think>", "").replace("</think>", "")
                .replace("<|im_start|>", "").replace("<|im_end|>", "");

            // Suppress empty planning narration ("I'll", "Let me", "I will")
            // Only on the very first output tokens
            if clean.len() < 50 {
                let lower = clean.trim().to_lowercase();
                if lower.starts_with("i'll ") || lower.starts_with("let me ") || lower.starts_with("i will ") {
                    // Don't print yet — buffer it in case it's just narration before a tool call
                    return;
                }
            }

            if !clean.is_empty() {
                buf.clear();
                if use_color_token {
                    print!("\x1b[37m{}\x1b[0m", clean);
                } else {
                    print!("{}", clean);
                }
                std::io::stdout().flush().ok();
            }
        }))
    } else {
        None
    };

    // Tool callbacks — show real-time progress in pretty format
    let use_color_tool = use_color;
    let on_tool_start: Option<pawan::agent::ToolStartCallback> = if is_json && stream {
        Some(Box::new(|name: &str| {
            let event = serde_json::json!({"type": "tool_start", "name": name});
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
        }))
    } else if !is_json {
        Some(Box::new(move |name: &str| {
            if use_color_tool {
                eprint!("\x1b[1;35m  ⚙ {}\x1b[0m", name);
            } else {
                eprint!("  > {}", name);
            }
        }))
    } else {
        None
    };

    let use_color_done = use_color;
    let on_tool_done: Option<pawan::agent::ToolCallback> = if is_json && stream {
        Some(Box::new(|tc: &pawan::agent::ToolCallRecord| {
            let event = serde_json::json!({
                "type": "tool_complete",
                "name": tc.name,
                "success": tc.success,
                "duration_ms": tc.duration_ms,
            });
            println!("{}", serde_json::to_string(&event).unwrap_or_default());
        }))
    } else if !is_json {
        Some(Box::new(move |tc: &pawan::agent::ToolCallRecord| {
            if use_color_done {
                if tc.success {
                    eprintln!(" \x1b[32m✓\x1b[0m \x1b[2m{}ms\x1b[0m", tc.duration_ms);
                } else {
                    eprintln!(" \x1b[31m✗\x1b[0m \x1b[2m{}ms\x1b[0m", tc.duration_ms);
                }
            } else {
                let icon = if tc.success { "ok" } else { "FAIL" };
                eprintln!(" [{}] {}ms", icon, tc.duration_ms);
            }
        }))
    } else {
        None
    };

    // Execute with timeout
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        agent.execute_with_callbacks(&prompt_text, on_token, on_tool_done, on_tool_start),
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
            std::process::exit(2);
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

            let clean_content = strip_thinking_tags(&response.content);
            let output = serde_json::json!({
                "success": true,
                "content": clean_content,
                "iterations": response.iterations,
                "tool_calls": tool_calls,
                "tool_call_count": response.tool_calls.len(),
                "usage": {
                    "prompt_tokens": response.usage.prompt_tokens,
                    "completion_tokens": response.usage.completion_tokens,
                    "total_tokens": response.usage.total_tokens,
                }
            });
            println!("{}", serde_json::to_string_pretty(&output).unwrap());
        }
        _ => {
            let content = strip_thinking_tags(&response.content);
            if !content.ends_with('\n') {
                println!();
            }

            // Summary footer
            let use_color = atty::is(atty::Stream::Stderr);
            let tc_count = response.tool_calls.len();
            let success_count = response.tool_calls.iter().filter(|t| t.success).count();
            let fail_count = tc_count - success_count;
            let total_ms: u64 = response.tool_calls.iter().map(|t| t.duration_ms).sum();

            if use_color {
                eprintln!();
                eprintln!("\x1b[1;36m┌─ summary\x1b[0m");
                eprintln!("\x1b[1;36m│\x1b[0m \x1b[33mIterations:\x1b[0m {} \x1b[2m│\x1b[0m \x1b[33mTools:\x1b[0m \x1b[32m{} ok\x1b[0m{} \x1b[2m│\x1b[0m \x1b[33mTime:\x1b[0m {}ms",
                    response.iterations,
                    success_count,
                    if fail_count > 0 { format!(" \x1b[31m{} fail\x1b[0m", fail_count) } else { String::new() },
                    total_ms,
                );
                if response.usage.total_tokens > 0 {
                    let budget = if response.usage.reasoning_tokens > 0 {
                        format!(" \x1b[2m(think:{} act:{})\x1b[0m", response.usage.reasoning_tokens, response.usage.action_tokens)
                    } else { String::new() };
                    eprintln!("\x1b[1;36m│\x1b[0m \x1b[33mTokens:\x1b[0m {}{}",
                        response.usage.total_tokens, budget);
                }
                if !response.tool_calls.is_empty() && verbose {
                    eprintln!("\x1b[1;36m│\x1b[0m \x1b[33mTool log:\x1b[0m");
                    for tc in &response.tool_calls {
                        let icon = if tc.success { "\x1b[32m✓\x1b[0m" } else { "\x1b[31m✗\x1b[0m" };
                        eprintln!("\x1b[1;36m│\x1b[0m   {} \x1b[1m{}\x1b[0m \x1b[2m{}ms\x1b[0m", icon, tc.name, tc.duration_ms);
                    }
                }
                eprintln!("\x1b[1;36m└─\x1b[0m");
            } else {
                eprintln!();
                eprintln!("── summary ──");
                eprintln!("Iterations: {} | Tools: {} ok{} | Time: {}ms",
                    response.iterations, success_count,
                    if fail_count > 0 { format!(", {} fail", fail_count) } else { String::new() },
                    total_ms);
                if response.usage.total_tokens > 0 {
                    let budget = if response.usage.reasoning_tokens > 0 {
                        format!(" (think:{} act:{})", response.usage.reasoning_tokens, response.usage.action_tokens)
                    } else { String::new() };
                    eprintln!("Tokens: {}{}", response.usage.total_tokens, budget);
                }
                if !response.tool_calls.is_empty() && verbose {
                    for tc in &response.tool_calls {
                        let s = if tc.success { "ok" } else { "FAIL" };
                        eprintln!("  [{}] {} {}ms", s, tc.name, tc.duration_ms);
                    }
                }
                eprintln!("─────────────");
            }

            // Only warn about no tool calls if the response seems incomplete
            // (empty content + no tools = likely a problem; content present = LLM answered directly)
            if response.tool_calls.is_empty() && response.content.trim().is_empty() {
                if use_color {
                    eprintln!("\x1b[33m⚠ No tool calls were made and response is empty.\x1b[0m");
                } else {
                    eprintln!("Warning: No tool calls were made and response is empty.");
                }
            }

            if verbose {
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

/// Strip <think>...</think> tags from thinking model responses
fn strip_thinking_tags(content: &str) -> String {
    let mut result = content.to_string();
    // Remove <think>...</think> blocks (including multiline)
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            let end = end + "</think>".len();
            result = format!("{}{}", &result[..start], &result[end..]);
        } else {
            // Unclosed <think> tag — remove from <think> to end
            result = result[..start].to_string();
            break;
        }
    }
    result.trim().to_string()
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

/// Run code formatting
async fn run_fmt(workspace: PathBuf, check: bool) -> Result<()> {
    use std::process::Command;

    println!("{}", "Pawan Format".green().bold());
    println!();

    // Run cargo fmt
    let fmt_args = if check {
        vec!["fmt", "--all", "--", "--check"]
    } else {
        vec!["fmt", "--all"]
    };

    println!("{} cargo {}", "Running:".cyan(), fmt_args.join(" "));
    let fmt_status = Command::new("cargo")
        .args(&fmt_args)
        .current_dir(&workspace)
        .status()
        .map_err(PawanError::Io)?;

    if fmt_status.success() {
        println!("{}", "  cargo fmt: OK".green());
    } else {
        println!("{}", "  cargo fmt: issues found".yellow());
        if check {
            return Ok(());
        }
    }

    // Run cargo clippy --fix (skip in check mode)
    if !check {
        println!("{} cargo clippy --fix", "Running:".cyan());
        let clippy_status = Command::new("cargo")
            .args(["clippy", "--fix", "--allow-dirty", "--allow-staged"])
            .current_dir(&workspace)
            .status()
            .map_err(PawanError::Io)?;

        if clippy_status.success() {
            println!("{}", "  cargo clippy --fix: OK".green());
        } else {
            println!("{}", "  cargo clippy --fix: some issues remain".yellow());
        }
    }

    println!();
    println!("{}", "Format complete.".green().bold());
    Ok(())
}

async fn run_bench() -> Result<()> {
    println!("{}", "Pawan Bench".green().bold());
    let nimakai = std::process::Command::new("which")
        .arg("nimakai")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "nimakai".to_string());
    if !std::path::Path::new(&nimakai).exists() {
        println!("nimakai not found in PATH. Install: nimble install nimakai");
        return Ok(());
    }
    let out = std::process::Command::new(nimakai).args(["bench", "--json"]).output();
    match out {
        Ok(o) => { println!("{}", String::from_utf8_lossy(&o.stdout)); }
        Err(e) => { println!("Error: {}", e); }
    }
    Ok(())
}

/// Send a notification via doltares relay
async fn run_notify(message: &str, channel: &str) -> Result<()> {
    let api_key = std::env::var("DOLTA_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        println!("{}", "DOLTA_API_KEY not set. Set it in .env or export it.".yellow());
        return Ok(());
    }

    let relay_url = std::env::var("DOLTARES_RELAY_URL")
        .unwrap_or_else(|_| "http://localhost:3100/api/deliver".to_string());

    let body = serde_json::json!({
        "channel": channel,
        "to": "last",
        "message": message,
    });

    let output = std::process::Command::new("curl")
        .args([
            "-s", "-X", "POST",
            &relay_url,
            "-H", "Content-Type: application/json",
            "-H", &format!("Authorization: Bearer {}", api_key),
            "-d", &serde_json::to_string(&body).unwrap_or_default(),
        ])
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if out.status.success() {
                println!("{} {}", "Sent:".green(), stdout.trim());
            } else {
                println!("{} {}", "Failed:".red(), stdout.trim());
            }
        }
        Err(e) => {
            println!("{} {}", "Error:".red(), e);
        }
    }

    Ok(())
}

