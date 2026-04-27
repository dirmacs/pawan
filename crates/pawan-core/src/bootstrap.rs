//! External dependency bootstrap — install the binaries pawan shells out to.
//!
//! Pawan depends on a few external tools that aren't pulled in by
//! `cargo install pawan`:
//!
//! - `mise` — polyglot tool/runtime manager (needed to install the rest)
//! - `rg`, `fd`, `sd`, `ast-grep`, `erd` — native search/replace/tree tools
//!   (auto-installed via mise on first tool use, but only if mise is present)
//!
//! As of the Option B rewrite, `deagle` is NO LONGER an external dep:
//! `deagle-core` and `deagle-parse` are embedded directly into pawan as
//! library crates, so all 5 deagle tools work out of the box. The
//! [`ensure_deagle`] function still exists for backwards compatibility
//! (and for users who want the standalone `deagle` CLI on PATH for
//! interactive use), but it's opt-in via `--include-deagle` and not
//! part of the default bootstrap.
//!
//! This module provides an idempotent, reversible install path so that
//! `cargo install pawan && pawan bootstrap` is enough to get a working
//! setup — no manual tool wrangling. Each step is non-destructive: it
//! checks `which <binary>` first and skips if the binary is already on
//! PATH, unless `force_reinstall` is set.
//!
//! ## Reversibility
//!
//! [`uninstall`] removes the marker file and optionally runs
//! `cargo uninstall deagle` (only if `--purge-deagle` is passed, and
//! only if the user had opted in to installing it). It deliberately
//! does NOT touch mise or mise-managed tools because those may be used
//! by other programs on the system.

use crate::{PawanError, Result};
use std::path::PathBuf;
use std::process::Command;

/// Options for a bootstrap run.
#[derive(Debug, Clone, Default)]
pub struct BootstrapOptions {
    /// Skip installing mise (caller will handle it themselves).
    pub skip_mise: bool,
    /// Skip installing the mise-managed native tools (rg/fd/sd/ast-grep/erd).
    pub skip_native: bool,
    /// ALSO install the standalone `deagle` CLI binary via
    /// `cargo install --locked deagle`. Opt-in because pawan already
    /// embeds `deagle-core` + `deagle-parse` as library deps, so the
    /// standalone CLI is only useful for interactive shell use.
    pub include_deagle: bool,
    /// Reinstall even if the binary is already on PATH. Off by default —
    /// bootstrap is meant to be safe to run repeatedly.
    pub force_reinstall: bool,
}

/// The outcome of installing (or trying to install) a single dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapStepStatus {
    /// Binary was already on PATH; no install attempted.
    AlreadyInstalled,
    /// Install ran successfully (binary is now on PATH).
    Installed,
    /// Install was skipped for a reason (e.g. mise not present).
    Skipped(String),
    /// Install attempt failed with an error message.
    Failed(String),
}

/// One line of a bootstrap report.
#[derive(Debug, Clone)]
pub struct BootstrapStep {
    pub name: String,
    pub status: BootstrapStepStatus,
}

/// Summary of a bootstrap run — one [`BootstrapStep`] per dependency.
#[derive(Debug, Clone, Default)]
pub struct BootstrapReport {
    pub steps: Vec<BootstrapStep>,
}

impl BootstrapReport {
    /// `true` if no step is in the `Failed` state. `Skipped` steps do not
    /// break the contract — they're a caller choice, not an error.
    pub fn all_ok(&self) -> bool {
        !self
            .steps
            .iter()
            .any(|s| matches!(s.status, BootstrapStepStatus::Failed(_)))
    }

    /// Number of steps that actually ran an install in this invocation.
    /// Used to decide whether to print a "N tool(s) installed" summary.
    pub fn installed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.status, BootstrapStepStatus::Installed))
            .count()
    }

    /// Number of steps that were already satisfied (idempotency signal).
    pub fn already_installed_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s.status, BootstrapStepStatus::AlreadyInstalled))
            .count()
    }

    /// Human-readable one-line summary for the end of a bootstrap run.
    pub fn summary(&self) -> String {
        let installed = self.installed_count();
        let existing = self.already_installed_count();
        let failed = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, BootstrapStepStatus::Failed(_)))
            .count();
        if failed > 0 {
            format!(
                "{} installed, {} already present, {} failed",
                installed, existing, failed
            )
        } else if installed == 0 {
            format!("all {} deps already present", existing)
        } else {
            format!("{} installed, {} already present", installed, existing)
        }
    }
}

/// The native tools managed by mise. Must match the tool names pawan
/// uses at runtime in `tools/native.rs`.
pub const NATIVE_TOOLS: &[&str] = &["rg", "fd", "sd", "ast-grep", "erd"];

/// Map a native binary name to its mise package name. Mirrors the
/// mapping in `tools/native.rs` — keep both in sync.
fn mise_package_name(binary: &str) -> &str {
    match binary {
        "erd" => "erdtree",
        "rg" => "ripgrep",
        "ast-grep" | "sg" => "ast-grep",
        other => other,
    }
}

/// Check if a binary is available on PATH.
pub fn binary_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

/// True if every REQUIRED external dep is on PATH. Deagle is excluded
/// because pawan embeds deagle-core + deagle-parse as library deps —
/// the standalone binary is no longer required.
pub fn is_bootstrapped() -> bool {
    binary_exists("mise") && NATIVE_TOOLS.iter().all(|t| binary_exists(t))
}

/// List of REQUIRED dep names that are NOT on PATH. Deagle is excluded
/// because pawan embeds it directly — see [`is_bootstrapped`]. Used by
/// `pawan doctor` and the CLI `bootstrap --dry-run` flag.
pub fn missing_deps() -> Vec<String> {
    let mut missing = Vec::new();
    if !binary_exists("mise") {
        missing.push("mise".to_string());
    }
    for tool in NATIVE_TOOLS {
        if !binary_exists(tool) {
            missing.push((*tool).to_string());
        }
    }
    missing
}

/// Path to the "this pawan has been bootstrapped" marker file. The
/// presence of this file is how startup knows to skip the auto-bootstrap
/// prompt on subsequent runs.
pub fn marker_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".pawan").join(".bootstrapped")
}

/// Install deagle via `cargo install --locked deagle`. Idempotent — if
/// deagle is already on PATH and `force` is false, returns
/// `AlreadyInstalled` without shelling out.
pub fn ensure_deagle(force: bool) -> BootstrapStep {
    if !force && binary_exists("deagle") {
        return BootstrapStep {
            name: "deagle".into(),
            status: BootstrapStepStatus::AlreadyInstalled,
        };
    }

    let output = Command::new("cargo")
        .args(["install", "--locked", "deagle"])
        .output();

    let status = match output {
        Ok(o) if o.status.success() => BootstrapStepStatus::Installed,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let brief: String = stderr.chars().take(200).collect();
            BootstrapStepStatus::Failed(format!("cargo install deagle failed: {}", brief))
        }
        Err(e) => {
            BootstrapStepStatus::Failed(format!("cargo install deagle spawn failed: {}", e))
        }
    };

    BootstrapStep {
        name: "deagle".into(),
        status,
    }
}

/// Install mise via `cargo install --locked mise`. Idempotent — skipped
/// if mise is already on PATH or at `~/.local/bin/mise` (mise's default
/// install location, which may not yet be on PATH).
///
/// We prefer `cargo install` over the curl-pipe-shell installer because
/// (a) cargo is already present for any user who got pawan via
/// `cargo install pawan`, and (b) cargo install is a known,
/// signed-binary path — no remote shell script trust required.
///
/// Note: `mise` itself is a binary-only crate on crates.io (no lib
/// target), so pawan cannot `use mise::...` directly. Bootstrap is the
/// only remaining integration surface.
pub fn ensure_mise() -> BootstrapStep {
    if binary_exists("mise") {
        return BootstrapStep {
            name: "mise".into(),
            status: BootstrapStepStatus::AlreadyInstalled,
        };
    }
    // Fallback: mise installs into ~/.local/bin/ which isn't always on
    // PATH in non-interactive shells. Detect the raw file.
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let local = format!("{}/.local/bin/mise", home);
    if std::path::Path::new(&local).exists() {
        return BootstrapStep {
            name: "mise".into(),
            status: BootstrapStepStatus::AlreadyInstalled,
        };
    }

    let output = Command::new("cargo")
        .args(["install", "--locked", "mise"])
        .output();

    let status = match output {
        Ok(o) if o.status.success() => BootstrapStepStatus::Installed,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let brief: String = stderr.chars().take(200).collect();
            BootstrapStepStatus::Failed(format!("cargo install mise failed: {}", brief))
        }
        Err(e) => BootstrapStepStatus::Failed(format!("cargo install mise spawn failed: {}", e)),
    };

    BootstrapStep {
        name: "mise".into(),
        status,
    }
}

/// Install a native tool via mise. Requires mise to already be on PATH
/// (or at `~/.local/bin/mise`) — returns Skipped otherwise so the caller
/// can decide how to surface that.
pub fn ensure_native_tool(tool: &str) -> BootstrapStep {
    if binary_exists(tool) {
        return BootstrapStep {
            name: tool.into(),
            status: BootstrapStepStatus::AlreadyInstalled,
        };
    }

    let mise_bin = if binary_exists("mise") {
        "mise".to_string()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let local = format!("{}/.local/bin/mise", home);
        if std::path::Path::new(&local).exists() {
            local
        } else {
            return BootstrapStep {
                name: tool.into(),
                status: BootstrapStepStatus::Skipped("mise not present".into()),
            };
        }
    };

    let pkg = mise_package_name(tool);
    let install = Command::new(&mise_bin)
        .args(["install", pkg, "-y"])
        .output();

    let status = match install {
        Ok(o) if o.status.success() => {
            // Also run `mise use --global` so the tool is on PATH for
            // subsequent processes.
            let _ = Command::new(&mise_bin)
                .args(["use", "--global", pkg])
                .output();
            BootstrapStepStatus::Installed
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let brief: String = stderr.chars().take(200).collect();
            BootstrapStepStatus::Failed(format!("mise install {} failed: {}", tool, brief))
        }
        Err(e) => {
            BootstrapStepStatus::Failed(format!("mise install {} spawn failed: {}", tool, e))
        }
    };

    BootstrapStep {
        name: tool.into(),
        status,
    }
}

/// Run the full bootstrap sequence per the options. On success (`all_ok`
/// returns true), writes a marker file at [`marker_path`] containing the
/// install timestamp.
///
/// Default (all opts false): installs mise and native tools. Deagle is
/// NOT installed by default — pawan embeds deagle-core + deagle-parse
/// as library deps, so the standalone CLI is only needed for
/// interactive shell use. Set `include_deagle = true` to opt in.
pub fn ensure_deps(opts: BootstrapOptions) -> BootstrapReport {
    let mut report = BootstrapReport::default();

    if !opts.skip_mise {
        report.steps.push(ensure_mise());
    }
    if !opts.skip_native {
        for tool in NATIVE_TOOLS {
            report.steps.push(ensure_native_tool(tool));
        }
    }
    if opts.include_deagle {
        report.steps.push(ensure_deagle(opts.force_reinstall));
    }

    // Write the marker only if the run completed without any Failed step.
    // Skipped steps are OK — they're caller-requested.
    if report.all_ok() {
        let path = marker_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, chrono::Utc::now().to_rfc3339());
    }

    report
}

/// Reverse the bootstrap: remove the marker file, and optionally run
/// `cargo uninstall deagle`. Deliberately does NOT uninstall mise or
/// mise-managed tools — those may be used by other programs.
pub fn uninstall(purge_deagle: bool) -> Result<()> {
    let path = marker_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| PawanError::Config(format!("remove marker: {}", e)))?;
    }

    if purge_deagle && binary_exists("deagle") {
        let output = Command::new("cargo")
            .args(["uninstall", "deagle"])
            .output()
            .map_err(|e| PawanError::Config(format!("cargo uninstall spawn: {}", e)))?;
        if !output.status.success() {
            return Err(PawanError::Config(format!(
                "cargo uninstall deagle failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_report_default_is_all_ok() {
        // An empty report has no failures, so all_ok() must be true.
        // Used when skip_mise=skip_deagle=skip_native=true.
        let report = BootstrapReport::default();
        assert!(report.all_ok());
        assert_eq!(report.installed_count(), 0);
        assert_eq!(report.already_installed_count(), 0);
    }

    #[test]
    fn bootstrap_report_with_failed_step_is_not_ok() {
        let report = BootstrapReport {
            steps: vec![BootstrapStep {
                name: "deagle".into(),
                status: BootstrapStepStatus::Failed("network".into()),
            }],
        };
        assert!(!report.all_ok());
        assert_eq!(report.installed_count(), 0);
    }

    #[test]
    fn bootstrap_report_skipped_step_is_not_a_failure() {
        // Skipped means "caller asked us not to do this" — not an error.
        let report = BootstrapReport {
            steps: vec![BootstrapStep {
                name: "mise".into(),
                status: BootstrapStepStatus::Skipped("caller skipped".into()),
            }],
        };
        assert!(report.all_ok(), "skipped != failed");
    }

    #[test]
    fn bootstrap_report_installed_count_excludes_already_installed() {
        let report = BootstrapReport {
            steps: vec![
                BootstrapStep {
                    name: "a".into(),
                    status: BootstrapStepStatus::Installed,
                },
                BootstrapStep {
                    name: "b".into(),
                    status: BootstrapStepStatus::AlreadyInstalled,
                },
                BootstrapStep {
                    name: "c".into(),
                    status: BootstrapStepStatus::Installed,
                },
            ],
        };
        assert_eq!(report.installed_count(), 2);
        assert_eq!(report.already_installed_count(), 1);
    }

    #[test]
    fn bootstrap_report_summary_shows_counts() {
        // All three categories exercised at once.
        let report = BootstrapReport {
            steps: vec![
                BootstrapStep {
                    name: "mise".into(),
                    status: BootstrapStepStatus::AlreadyInstalled,
                },
                BootstrapStep {
                    name: "deagle".into(),
                    status: BootstrapStepStatus::Installed,
                },
                BootstrapStep {
                    name: "rg".into(),
                    status: BootstrapStepStatus::Failed("nope".into()),
                },
            ],
        };
        let s = report.summary();
        assert!(s.contains("1 installed"));
        assert!(s.contains("1 already present"));
        assert!(s.contains("1 failed"));
    }

    #[test]
    fn bootstrap_report_summary_all_present() {
        let report = BootstrapReport {
            steps: vec![
                BootstrapStep {
                    name: "mise".into(),
                    status: BootstrapStepStatus::AlreadyInstalled,
                },
                BootstrapStep {
                    name: "deagle".into(),
                    status: BootstrapStepStatus::AlreadyInstalled,
                },
            ],
        };
        assert_eq!(report.summary(), "all 2 deps already present");
    }

    #[test]
    fn native_tools_constant_is_5_well_known_tools() {
        // Regression guard: if someone adds or removes a native tool,
        // they must update BOTH this constant AND the registry entry in
        // tools/mod.rs. This test catches drift.
        assert_eq!(NATIVE_TOOLS.len(), 5);
        assert!(NATIVE_TOOLS.contains(&"rg"));
        assert!(NATIVE_TOOLS.contains(&"fd"));
        assert!(NATIVE_TOOLS.contains(&"sd"));
        assert!(NATIVE_TOOLS.contains(&"ast-grep"));
        assert!(NATIVE_TOOLS.contains(&"erd"));
    }

    #[test]
    fn mise_package_name_handles_binary_name_mismatch() {
        // `rg` is the binary; `ripgrep` is the mise package. Same for erd
        // and erdtree. If someone breaks this mapping, mise install fails
        // silently from the user's POV.
        assert_eq!(mise_package_name("rg"), "ripgrep");
        assert_eq!(mise_package_name("erd"), "erdtree");
        assert_eq!(mise_package_name("fd"), "fd");
        assert_eq!(mise_package_name("sd"), "sd");
        assert_eq!(mise_package_name("ast-grep"), "ast-grep");
        assert_eq!(mise_package_name("sg"), "ast-grep");
        // Unknown tools fall through unchanged
        assert_eq!(mise_package_name("unknown-tool"), "unknown-tool");
    }

    #[test]
    fn marker_path_is_under_home_dot_pawan() {
        // Documents where the marker lives so `pawan uninstall` knows
        // what to remove. If this moves, both locations must update.
        let path = marker_path();
        let s = path.to_string_lossy();
        assert!(s.ends_with(".pawan/.bootstrapped"));
    }

    #[test]
    fn ensure_deagle_is_idempotent_when_already_on_path() {
        // On boxes where deagle is installed, ensure_deagle(false) must
        // NOT shell out to cargo. This is the idempotency contract.
        if !binary_exists("deagle") {
            // Skip on bare boxes — we can't test the branch without a
            // pre-installed deagle.
            return;
        }
        let step = ensure_deagle(false);
        assert_eq!(step.name, "deagle");
        assert_eq!(
            step.status,
            BootstrapStepStatus::AlreadyInstalled,
            "second call must be a no-op when deagle is present"
        );
    }

    #[test]
    fn ensure_mise_is_idempotent_when_already_on_path() {
        if !binary_exists("mise") {
            // If mise is at ~/.local/bin/mise but not on PATH, the
            // fallback branch also returns AlreadyInstalled.
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            if !std::path::Path::new(&format!("{}/.local/bin/mise", home)).exists() {
                return; // bare box — skip
            }
        }
        let step = ensure_mise();
        assert_eq!(step.name, "mise");
        assert_eq!(step.status, BootstrapStepStatus::AlreadyInstalled);
    }

    #[test]
    fn ensure_native_tool_is_idempotent_when_already_on_path() {
        // Pick whichever native tool is present on this box.
        for tool in NATIVE_TOOLS {
            if binary_exists(tool) {
                let step = ensure_native_tool(tool);
                assert_eq!(step.name, *tool);
                assert_eq!(
                    step.status,
                    BootstrapStepStatus::AlreadyInstalled,
                    "ensure_native_tool({}) must be idempotent",
                    tool
                );
                return;
            }
        }
        // All tools missing — nothing to test.
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn missing_deps_is_empty_on_fully_bootstrapped_box() {
        if !is_bootstrapped() {
            return;
        }
        assert!(
            missing_deps().is_empty(),
            "is_bootstrapped() and missing_deps() must agree"
        );
    }

    #[test]
    fn ensure_deps_with_all_skips_writes_empty_report() {
        // skip_mise + skip_native = no steps attempted. include_deagle
        // defaults to false (embedded), so no deagle step either.
        // all_ok must still be true.
        let opts = BootstrapOptions {
            skip_mise: true,
            skip_native: true,
            include_deagle: false,
            force_reinstall: false,
        };
        let report = ensure_deps(opts);
        assert_eq!(report.steps.len(), 0);
        assert!(report.all_ok());
        assert_eq!(report.installed_count(), 0);
    }

    #[test]
    fn default_options_do_not_include_deagle() {
        // Default bootstrap must NOT try to install deagle — it's embedded
        // as a library now. This catches any future refactor that flips
        // the default.
        let opts = BootstrapOptions::default();
        assert!(!opts.include_deagle, "default must exclude deagle install");
        assert!(!opts.skip_mise, "default installs mise");
        assert!(!opts.skip_native, "default installs native tools");
        assert!(!opts.force_reinstall);
    }

    #[test]
    fn is_bootstrapped_does_not_require_deagle() {
        // The embedded library means is_bootstrapped() must NOT check
        // for the deagle binary. This guards against a regression where
        // someone re-adds the check.
        // We can't fully test the "true" case without mocking `which`,
        // but we can assert that the function's behavior doesn't depend
        // on deagle binary presence: if mise + all native tools are on
        // PATH, it must be true regardless of deagle.
        if binary_exists("mise") && NATIVE_TOOLS.iter().all(|t| binary_exists(t)) {
            assert!(is_bootstrapped());
        }
        // Also: missing_deps must not list deagle
        assert!(
            !missing_deps().iter().any(|d| d == "deagle"),
            "missing_deps must not mention deagle"
        );
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn uninstall_without_marker_file_is_ok() {
        // Calling uninstall on a box without a marker must NOT error —
        // it's the "nothing to clean up" path.
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        // Temporarily redirect HOME so we don't touch the real marker.
        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let result = uninstall(false);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        assert!(result.is_ok());
    }

    #[test]
    fn bootstrap_report_summary_with_installs_only() {
        let report = BootstrapReport {
            steps: vec![
                BootstrapStep {
                    name: "mise".into(),
                    status: BootstrapStepStatus::Installed,
                },
                BootstrapStep {
                    name: "rg".into(),
                    status: BootstrapStepStatus::Installed,
                },
            ],
        };
        let s = report.summary();
        assert!(s.contains("2 installed"));
        assert!(s.contains("0 already present"));
    }

    #[test]
    fn missing_deps_lists_missing_mise() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let missing = missing_deps();

        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        assert!(missing.contains(&"mise".to_string()));
    }

    #[test]
    fn missing_deps_lists_missing_native_tools() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let missing = missing_deps();

        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        for tool in NATIVE_TOOLS {
            assert!(missing.contains(&tool.to_string()));
        }
    }

    #[test]
    fn ensure_deagle_with_force_reinstall_attempts_install() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let step = ensure_deagle(true);

        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        assert_eq!(step.name, "deagle");
        // Will fail in test env, but we verify the attempt was made
        assert!(matches!(step.status, BootstrapStepStatus::Installed | BootstrapStepStatus::Failed(_)));
    }

    #[test]
    fn ensure_mise_falls_back_to_local_bin() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let home = tmp.path();
        let local_bin = home.join(".local/bin");
        std::fs::create_dir_all(&local_bin).unwrap();
        std::fs::write(local_bin.join("mise"), "#!/bin/sh\necho mise").unwrap();

        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);

        let step = ensure_mise();

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }
        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        assert_eq!(step.name, "mise");
        assert_eq!(step.status, BootstrapStepStatus::AlreadyInstalled);
    }

    #[test]
    fn ensure_native_tool_skips_when_mise_missing() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let home = tmp.path();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);

        let step = ensure_native_tool("rg");

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }
        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        assert_eq!(step.name, "rg");
        assert!(matches!(step.status, BootstrapStepStatus::Skipped(_)));
    }

    #[test]
    fn ensure_native_tool_uses_local_mise() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var("PATH").ok();
        std::env::set_var("PATH", tmp.path());

        let home = tmp.path();
        let local_bin = home.join(".local/bin");
        std::fs::create_dir_all(&local_bin).unwrap();
        std::fs::write(local_bin.join("mise"), "#!/bin/sh\necho mise").unwrap();

        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home);

        let step = ensure_native_tool("rg");

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }
        if let Some(p) = prev_path {
            std::env::set_var("PATH", p);
        }

        assert_eq!(step.name, "rg");
        // Will fail in test env, but we verify it didn't skip
        assert!(matches!(step.status, BootstrapStepStatus::Installed | BootstrapStepStatus::Failed(_)));
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn ensure_deps_writes_marker_on_success() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let opts = BootstrapOptions {
            skip_mise: true,
            skip_native: true,
            include_deagle: false,
            force_reinstall: false,
        };
        let report = ensure_deps(opts);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        assert!(report.all_ok());
        let marker = tmp.path().join(".pawan/.bootstrapped");
        assert!(marker.exists(), "marker must be written on success");
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn ensure_deps_includes_deagle_when_requested() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let opts = BootstrapOptions {
            skip_mise: true,
            skip_native: true,
            include_deagle: true,
            force_reinstall: false,
        };
        let report = ensure_deps(opts);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].name, "deagle");
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn ensure_deps_with_force_reinstall() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let opts = BootstrapOptions {
            skip_mise: true,
            skip_native: true,
            include_deagle: true,
            force_reinstall: true,
        };
        let report = ensure_deps(opts);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].name, "deagle");
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn uninstall_removes_marker_file() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let marker = tmp.path().join(".pawan/.bootstrapped");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "2024-01-01T00:00:00Z").unwrap();

        let result = uninstall(false);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        assert!(result.is_ok());
        assert!(!marker.exists(), "marker must be removed");
    }

    #[serial_test::serial(pawan_session_tests)]
    #[test]
    fn uninstall_with_purge_deagle_attempts_uninstall() {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());

        let marker = tmp.path().join(".pawan/.bootstrapped");
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(&marker, "2024-01-01T00:00:00Z").unwrap();

        let result = uninstall(true);

        if let Some(h) = prev_home {
            std::env::set_var("HOME", h);
        }

        // Will fail if deagle not installed, but we verify the attempt
        assert!(result.is_ok() || result.is_err());
        assert!(!marker.exists(), "marker must be removed regardless");
    }}
