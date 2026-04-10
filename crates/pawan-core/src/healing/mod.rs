//! Self-healing module for Pawan
//!
//! This module provides automated fixing capabilities for:
//! - Compilation errors (`rustc` errors)
//! - Clippy warnings
//! - Test failures
//! - Missing documentation

use crate::config::HealingConfig;
use crate::{PawanError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

/// Shared cargo command runner with concurrent stdout/stderr reads and 5-minute timeout.
async fn run_cargo_command(workspace_root: &Path, args: &[&str]) -> Result<String> {
    let child = Command::new("cargo")
        .args(args)
        .current_dir(workspace_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(PawanError::Io)?;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| PawanError::Timeout("cargo command timed out after 5 minutes".into()))?
    .map_err(PawanError::Io)?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(format!("{}\n{}", stdout, stderr))
}

/// A compilation diagnostic (error or warning)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The type of diagnostic
    pub kind: DiagnosticKind,
    /// The diagnostic message
    pub message: String,
    /// File path where the issue is
    pub file: Option<PathBuf>,
    /// Line number (1-indexed)
    pub line: Option<usize>,
    /// Column number (1-indexed)  
    pub column: Option<usize>,
    /// The error/warning code (e.g., E0425)
    pub code: Option<String>,
    /// Suggested fix from the compiler
    pub suggestion: Option<String>,
    /// Full raw output for context
    pub raw: String,
}

/// Type of diagnostic
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    Error,
    Warning,
    Note,
    Help,
}

/// Result from a healing operation containing remaining issues and a summary.
#[derive(Debug)]
pub struct HealingResult {
    /// Remaining unfixed issues
    pub remaining: Vec<Diagnostic>,
    /// Description of what was done
    pub summary: String,
}

/// Compiler error fixer — parses cargo check output (JSON + text fallback) into Diagnostics.
pub struct CompilerFixer {
    workspace_root: PathBuf,
}

impl CompilerFixer {
    /// Create a new CompilerFixer
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Parse cargo check output into diagnostics
    ///
    /// This method supports both JSON output (from `cargo check --message-format=json`)
    /// and text output formats.
    ///
    /// # Arguments
    /// * `output` - The output from cargo check
    ///
    /// # Returns
    /// A vector of Diagnostic objects representing compilation errors and warnings
    pub fn parse_diagnostics(&self, output: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Parse the JSON output from cargo check --message-format=json
        for line in output.lines() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(msg) = json.get("message") {
                    let diagnostic = self.parse_diagnostic_message(msg, line);
                    if let Some(d) = diagnostic {
                        diagnostics.push(d);
                    }
                }
            }
        }

        // If no JSON output, try parsing text format
        if diagnostics.is_empty() {
            diagnostics = self.parse_text_diagnostics(output);
        }

        diagnostics
    }

    /// Parse a JSON diagnostic message
    fn parse_diagnostic_message(&self, msg: &serde_json::Value, raw: &str) -> Option<Diagnostic> {
        let level = msg.get("level")?.as_str()?;
        let message = msg.get("message")?.as_str()?.to_string();

        let kind = match level {
            "error" => DiagnosticKind::Error,
            "warning" => DiagnosticKind::Warning,
            "note" => DiagnosticKind::Note,
            "help" => DiagnosticKind::Help,
            _ => return None,
        };

        // Skip ICE messages and internal errors
        if message.contains("internal compiler error") {
            return None;
        }

        // Extract code
        let code = msg
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        // Extract primary span
        let spans = msg.get("spans")?.as_array()?;
        let primary_span = spans.iter().find(|s| {
            s.get("is_primary")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });

        let (file, line, column) = if let Some(span) = primary_span {
            let file = span
                .get("file_name")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);
            let line = span
                .get("line_start")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let column = span
                .get("column_start")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            (file, line, column)
        } else {
            (None, None, None)
        };

        // Extract suggestion
        let suggestion = msg
            .get("children")
            .and_then(|c| c.as_array())
            .and_then(|children| {
                children.iter().find_map(|child| {
                    let level = child.get("level")?.as_str()?;
                    if level == "help" {
                        let help_msg = child.get("message")?.as_str()?;
                        // Look for suggested replacement
                        if let Some(spans) = child.get("spans").and_then(|s| s.as_array()) {
                            for span in spans {
                                if let Some(replacement) =
                                    span.get("suggested_replacement").and_then(|v| v.as_str())
                                {
                                    return Some(format!("{}: {}", help_msg, replacement));
                                }
                            }
                        }
                        return Some(help_msg.to_string());
                    }
                    None
                })
            });

        Some(Diagnostic {
            kind,
            message,
            file,
            line,
            column,
            code,
            suggestion,
            raw: raw.to_string(),
        })
    }

    /// Parse text format diagnostics (fallback)
    fn parse_text_diagnostics(&self, output: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let mut current_diagnostic: Option<Diagnostic> = None;

        for line in output.lines() {
            // Match pattern: error[E0425]: cannot find value `x` in this scope
            if line.starts_with("error") || line.starts_with("warning") {
                // Save previous diagnostic
                if let Some(d) = current_diagnostic.take() {
                    diagnostics.push(d);
                }

                let kind = if line.starts_with("error") {
                    DiagnosticKind::Error
                } else {
                    DiagnosticKind::Warning
                };

                // Extract code like E0425
                let code = line
                    .find('[')
                    .and_then(|start| line.find(']').map(|end| line[start + 1..end].to_string()));

                // Extract message
                let message = if let Some(colon_pos) = line.find("]: ") {
                    line[colon_pos + 3..].to_string()
                } else if let Some(colon_pos) = line.find(": ") {
                    line[colon_pos + 2..].to_string()
                } else {
                    line.to_string()
                };

                current_diagnostic = Some(Diagnostic {
                    kind,
                    message,
                    file: None,
                    line: None,
                    column: None,
                    code,
                    suggestion: None,
                    raw: line.to_string(),
                });
            }
            // Match pattern: --> src/main.rs:10:5
            else if line.trim().starts_with("-->") {
                if let Some(ref mut d) = current_diagnostic {
                    let path_part = line.trim().trim_start_matches("-->").trim();
                    // Parse file:line:column (column may be absent)
                    let parts: Vec<&str> = path_part.rsplitn(3, ':').collect();
                    match parts.len() {
                        3 => {
                            // file:line:column
                            d.column = parts[0].parse().ok();
                            d.line = parts[1].parse().ok();
                            d.file = Some(PathBuf::from(parts[2]));
                        }
                        2 => {
                            // file:line (no column)
                            d.line = parts[0].parse().ok();
                            d.file = Some(PathBuf::from(parts[1]));
                        }
                        _ => {}
                    }
                }
            }
            // Match help suggestions
            else if line.trim().starts_with("help:") {
                if let Some(ref mut d) = current_diagnostic {
                    let suggestion = line.trim().trim_start_matches("help:").trim();
                    d.suggestion = Some(suggestion.to_string());
                }
            }
        }

        // Don't forget the last one
        if let Some(d) = current_diagnostic {
            diagnostics.push(d);
        }

        diagnostics
    }

    /// Run cargo check and get diagnostics
    pub async fn check(&self) -> Result<Vec<Diagnostic>> {
        let output = run_cargo_command(&self.workspace_root, &["check", "--message-format=json"]).await?;
        Ok(self.parse_diagnostics(&output))
    }
}

/// Clippy warning fixer — runs clippy and filters to warnings only.
pub struct ClippyFixer {
    workspace_root: PathBuf,
}

impl ClippyFixer {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Run clippy and get warnings
    pub async fn check(&self) -> Result<Vec<Diagnostic>> {
        let output = run_cargo_command(
            &self.workspace_root,
            &["clippy", "--message-format=json", "--", "-W", "clippy::all"],
        ).await?;
        let fixer = CompilerFixer::new(self.workspace_root.clone());
        let mut diagnostics = fixer.parse_diagnostics(&output);
        diagnostics.retain(|d| d.kind == DiagnosticKind::Warning);
        Ok(diagnostics)
    }
}

/// Test failure fixer — parses cargo test output to identify and locate failed tests.
pub struct TestFixer {
    workspace_root: PathBuf,
}

/// A failed test with name, module, failure output, and optional source location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedTest {
    /// Test name (full path)
    pub name: String,
    /// Test module path
    pub module: String,
    /// Failure message/output
    pub failure: String,
    /// Location of the test
    pub file: Option<PathBuf>,
    /// Line number
    pub line: Option<usize>,
}

impl TestFixer {
    /// Create a new TestFixer
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Run tests and get failures
    pub async fn check(&self) -> Result<Vec<FailedTest>> {
        let output = run_cargo_command(
            &self.workspace_root,
            &["test", "--no-fail-fast", "--", "--nocapture"],
        ).await?;
        Ok(self.parse_test_output(&output))
    }

    /// Parse test output for failures
    fn parse_test_output(&self, output: &str) -> Vec<FailedTest> {
        let mut failures = Vec::new();
        let mut in_failures_section = false;
        let mut current_test: Option<String> = None;
        let mut current_output = String::new();

        for line in output.lines() {
            // Detect failures section
            if line.contains("failures:") && !line.contains("test result:") {
                in_failures_section = true;
                continue;
            }

            // End of failures section
            if in_failures_section && line.starts_with("test result:") {
                // Save last failure
                if let Some(test_name) = current_test.take() {
                    failures.push(FailedTest {
                        name: test_name.clone(),
                        module: self.extract_module(&test_name),
                        failure: current_output.trim().to_string(),
                        file: None,
                        line: None,
                    });
                }
                break;
            }

            // Detect individual test failure
            if line.starts_with("---- ") && line.ends_with(" stdout ----") {
                // Save previous failure
                if let Some(test_name) = current_test.take() {
                    failures.push(FailedTest {
                        name: test_name.clone(),
                        module: self.extract_module(&test_name),
                        failure: current_output.trim().to_string(),
                        file: None,
                        line: None,
                    });
                }

                // Start new failure
                let test_name = line
                    .trim_start_matches("---- ")
                    .trim_end_matches(" stdout ----")
                    .to_string();
                current_test = Some(test_name);
                current_output.clear();
            } else if current_test.is_some() {
                current_output.push_str(line);
                current_output.push('\n');
            }
        }

        // Also look for simple FAILED lines (but skip "test result:" summary lines)
        for line in output.lines() {
            if line.contains("FAILED") && line.starts_with("test ") && !line.starts_with("test result:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let test_name = parts[1].trim_end_matches(" ...");

                    // Check if we already have this failure
                    if !failures.iter().any(|f| f.name == test_name) {
                        failures.push(FailedTest {
                            name: test_name.to_string(),
                            module: self.extract_module(test_name),
                            failure: line.to_string(),
                            file: None,
                            line: None,
                        });
                    }
                }
            }
        }

        failures
    }

    /// Extract module path from test name
    fn extract_module(&self, test_name: &str) -> String {
        if let Some(pos) = test_name.rfind("::") {
            test_name[..pos].to_string()
        } else {
            String::new()
        }
    }
}

/// Healer — coordinates CompilerFixer, ClippyFixer, and TestFixer for self-healing.
pub struct Healer {
    #[allow(dead_code)]
    workspace_root: PathBuf,
    config: HealingConfig,
    compiler_fixer: CompilerFixer,
    clippy_fixer: ClippyFixer,
    test_fixer: TestFixer,
}

impl Healer {
    /// Create a new Healer
    pub fn new(workspace_root: PathBuf, config: HealingConfig) -> Self {
        Self {
            compiler_fixer: CompilerFixer::new(workspace_root.clone()),
            clippy_fixer: ClippyFixer::new(workspace_root.clone()),
            test_fixer: TestFixer::new(workspace_root.clone()),
            workspace_root,
            config,
        }
    }

    /// Get all diagnostics (errors and warnings) from the workspace
    ///
    /// This method runs cargo check and clippy (if configured) to collect
    /// compilation errors and warnings.
    ///
    /// # Returns
    /// A vector of Diagnostic objects, or an error if the checks fail to run
    pub async fn get_diagnostics(&self) -> Result<Vec<Diagnostic>> {
        let mut all = Vec::new();

        if self.config.fix_errors {
            all.extend(self.compiler_fixer.check().await?);
        }

        if self.config.fix_warnings {
            all.extend(self.clippy_fixer.check().await?);
        }

        Ok(all)
    }

    /// Get all failed tests from the workspace
    ///
    /// This method runs cargo test and collects information about failed tests.
    ///
    /// # Returns
    /// A vector of FailedTest objects, or an error if the tests fail to run
    pub async fn get_failed_tests(&self) -> Result<Vec<FailedTest>> {
        if self.config.fix_tests {
            self.test_fixer.check().await
        } else {
            Ok(Vec::new())
        }
    }

    /// Count total issues concurrently: (errors, warnings, failed_tests).
    pub async fn count_issues(&self) -> Result<(usize, usize, usize)> {
        let (diagnostics, tests) = tokio::join!(
            self.get_diagnostics(),
            self.get_failed_tests(),
        );
        let diagnostics = diagnostics?;
        let tests = tests?;

        let errors = diagnostics
            .iter()
            .filter(|d| d.kind == DiagnosticKind::Error)
            .count();
        let warnings = diagnostics
            .iter()
            .filter(|d| d.kind == DiagnosticKind::Warning)
            .count();
        let failed_tests = tests.len();

        Ok((errors, warnings, failed_tests))
    }

    /// Format diagnostics for LLM prompt
    ///
    /// This method formats compilation diagnostics into a structured format
    /// suitable for inclusion in an LLM prompt.
    ///
    /// # Arguments
    /// * `diagnostics` - A slice of Diagnostic objects to format
    ///
    /// # Returns
    /// A formatted string ready for use in an LLM prompt
    pub fn format_diagnostics_for_prompt(&self, diagnostics: &[Diagnostic]) -> String {
        let mut output = String::new();

        for (i, d) in diagnostics.iter().enumerate() {
            output.push_str(&format!("\n### Issue {}\n", i + 1));
            output.push_str(&format!("Type: {:?}\n", d.kind));

            if let Some(ref code) = d.code {
                output.push_str(&format!("Code: {}\n", code));
            }

            output.push_str(&format!("Message: {}\n", d.message));

            if let Some(ref file) = d.file {
                output.push_str(&format!(
                    "Location: {}:{}:{}\n",
                    file.display(),
                    d.line.unwrap_or(0),
                    d.column.unwrap_or(0)
                ));
            }

            if let Some(ref suggestion) = d.suggestion {
                output.push_str(&format!("Suggestion: {}\n", suggestion));
            }
        }

        output
    }

    /// Format failed tests for LLM prompt
    pub fn format_tests_for_prompt(&self, tests: &[FailedTest]) -> String {
        let mut output = String::new();

        for (i, test) in tests.iter().enumerate() {
            output.push_str(&format!("\n### Failed Test {}\n", i + 1));
            output.push_str(&format!("Name: {}\n", test.name));
            output.push_str(&format!("Module: {}\n", test.module));
            output.push_str(&format!("Failure:\n```\n{}\n```\n", test.failure));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_text_diagnostic() {
        let output = r#"error[E0425]: cannot find value `x` in this scope
   --> src/main.rs:10:5
    |
10  |     x
    |     ^ not found in this scope
"#;

        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Error);
        assert_eq!(diagnostics[0].code, Some("E0425".to_string()));
        assert!(diagnostics[0].message.contains("cannot find value"));
    }

    #[test]
    fn test_extract_module() {
        let fixer = TestFixer::new(PathBuf::from("."));

        assert_eq!(
            fixer.extract_module("crate::module::tests::test_foo"),
            "crate::module::tests"
        );
        assert_eq!(fixer.extract_module("test_foo"), "");
    }

    #[test]
    fn test_parse_text_diagnostic_with_location() {
        let output = "error[E0308]: mismatched types\n   --> src/lib.rs:42:10\n";
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(diagnostics[0].line, Some(42));
        assert_eq!(diagnostics[0].column, Some(10));
    }

    #[test]
    fn test_parse_text_diagnostic_file_line_only() {
        // Some diagnostics omit the column
        let output = "warning: unused variable\n   --> src/main.rs:5\n";
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file, Some(PathBuf::from("src/main.rs")));
        assert_eq!(diagnostics[0].line, Some(5));
        assert_eq!(diagnostics[0].column, None);
    }

    #[test]
    fn test_parse_text_diagnostic_warning() {
        let output = "warning: unused variable `x`\n   --> src/lib.rs:3:5\n";
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Warning);
        assert!(diagnostics[0].message.contains("unused variable"));
    }

    #[test]
    fn test_parse_text_diagnostic_with_help() {
        let output = "error[E0425]: cannot find value `x`\n   --> src/main.rs:10:5\nhelp: consider importing this\n";
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].suggestion, Some("consider importing this".to_string()));
    }

    #[test]
    fn test_parse_text_multiple_diagnostics() {
        let output = "error[E0425]: first error\n   --> a.rs:1:1\nerror[E0308]: second error\n   --> b.rs:2:2\n";
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics(output);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].code, Some("E0425".to_string()));
        assert_eq!(diagnostics[1].code, Some("E0308".to_string()));
        assert_eq!(diagnostics[0].file, Some(PathBuf::from("a.rs")));
        assert_eq!(diagnostics[1].file, Some(PathBuf::from("b.rs")));
    }

    #[test]
    fn test_parse_text_empty_output() {
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_text_diagnostics("");
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_parse_json_diagnostic() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"error","message":"unused","code":{"code":"E0001"},"spans":[{"file_name":"src/lib.rs","line_start":5,"column_start":3,"is_primary":true}],"children":[]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(json_line);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Error);
        assert_eq!(diagnostics[0].file, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(diagnostics[0].line, Some(5));
        assert_eq!(diagnostics[0].column, Some(3));
    }

    #[test]
    fn test_parse_json_skips_ice() {
        let json_line = r#"{"reason":"compiler-message","message":{"level":"error","message":"internal compiler error: something broke","spans":[],"children":[]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(json_line);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_parse_test_output_failures() {
        let output = "---- tests::test_add stdout ----\nthread panicked at 'assertion failed'\n\nfailures:\n    tests::test_add\n\ntest result: FAILED. 1 passed; 1 failed;\n";
        let fixer = TestFixer::new(PathBuf::from("."));
        let failures = fixer.parse_test_output(output);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].name, "tests::test_add");
        assert_eq!(failures[0].module, "tests");
        assert!(failures[0].failure.contains("assertion failed"));
    }

    #[test]
    fn test_parse_test_output_no_failures() {
        let output = "running 5 tests\ntest result: ok. 5 passed; 0 failed;\n";
        let fixer = TestFixer::new(PathBuf::from("."));
        let failures = fixer.parse_test_output(output);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_parse_test_output_simple_failed_line() {
        // Use only the "test X ... FAILED" line without "test result: FAILED"
        let output = "test my_module::test_thing ... FAILED\n";
        let fixer = TestFixer::new(PathBuf::from("."));
        let failures = fixer.parse_test_output(output);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].name, "my_module::test_thing");
        assert_eq!(failures[0].module, "my_module");
    }

    #[test]
    fn test_format_diagnostics_for_prompt() {
        let healer = Healer::new(PathBuf::from("."), HealingConfig::default());
        let diagnostics = vec![Diagnostic {
            kind: DiagnosticKind::Error,
            message: "unused variable".into(),
            file: Some(PathBuf::from("src/lib.rs")),
            line: Some(10),
            column: Some(5),
            code: Some("E0001".into()),
            suggestion: Some("remove it".into()),
            raw: String::new(),
        }];
        let output = healer.format_diagnostics_for_prompt(&diagnostics);
        assert!(output.contains("Issue 1"));
        assert!(output.contains("E0001"));
        assert!(output.contains("unused variable"));
        assert!(output.contains("src/lib.rs:10:5"));
        assert!(output.contains("remove it"));
    }

    #[test]
    fn test_format_tests_for_prompt() {
        let healer = Healer::new(PathBuf::from("."), HealingConfig::default());
        let tests = vec![FailedTest {
            name: "tests::test_foo".into(),
            module: "tests".into(),
            failure: "assertion failed".into(),
            file: None,
            line: None,
        }];
        let output = healer.format_tests_for_prompt(&tests);
        assert!(output.contains("Failed Test 1"));
        assert!(output.contains("tests::test_foo"));
        assert!(output.contains("assertion failed"));
    }

    #[test]
    fn test_parse_json_note_and_help_levels() {
        // Note and Help are valid diagnostic kinds — should not be filtered out.
        let note_line = r#"{"reason":"compiler-message","message":{"level":"note","message":"for more info, see E0001","spans":[],"children":[]}}"#;
        let help_line = r#"{"reason":"compiler-message","message":{"level":"help","message":"consider borrowing","spans":[],"children":[]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let combined = format!("{}\n{}", note_line, help_line);
        let diagnostics = fixer.parse_diagnostics(&combined);
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Note);
        assert_eq!(diagnostics[1].kind, DiagnosticKind::Help);
        assert_eq!(diagnostics[0].file, None);
        assert_eq!(diagnostics[0].line, None);
    }

    #[test]
    fn test_parse_json_unknown_level_is_filtered() {
        // An unrecognized level like "trace" or "debug" should be skipped entirely.
        let line = r#"{"reason":"compiler-message","message":{"level":"trace","message":"verbose info","spans":[],"children":[]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(line);
        assert!(
            diagnostics.is_empty(),
            "unknown level should be filtered, got {} diagnostics",
            diagnostics.len()
        );
    }

    #[test]
    fn test_parse_json_suggestion_with_replacement() {
        // children[].spans[].suggested_replacement should be combined into the
        // suggestion field as "help_msg: replacement_text".
        let json = r#"{"reason":"compiler-message","message":{"level":"error","message":"missing semicolon","code":{"code":"E0001"},"spans":[{"file_name":"src/foo.rs","line_start":3,"column_start":10,"is_primary":true}],"children":[{"level":"help","message":"add semicolon","spans":[{"suggested_replacement":";"}]}]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(json);
        assert_eq!(diagnostics.len(), 1);
        let d = &diagnostics[0];
        assert!(d.suggestion.is_some(), "suggestion should be populated");
        let suggestion = d.suggestion.as_ref().unwrap();
        assert!(
            suggestion.contains("add semicolon"),
            "suggestion missing help text: {}",
            suggestion
        );
        assert!(
            suggestion.contains(";"),
            "suggestion missing replacement: {}",
            suggestion
        );
    }

    #[test]
    fn test_parse_json_no_primary_span() {
        // When no span has is_primary=true, file/line/column should all be None.
        let json = r#"{"reason":"compiler-message","message":{"level":"error","message":"no primary span","code":null,"spans":[{"file_name":"src/x.rs","line_start":1,"is_primary":false}],"children":[]}}"#;
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(json);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file, None);
        assert_eq!(diagnostics[0].line, None);
        assert_eq!(diagnostics[0].column, None);
    }

    #[test]
    fn test_parse_mixed_json_and_text_prefers_json() {
        // When JSON parsing succeeds on at least one line, text fallback must NOT
        // fire — otherwise a single good JSON line would be augmented with
        // potentially wrong text-parsed versions of surrounding lines.
        let mixed = format!(
            "{}\nerror[E0999]: should not be double-parsed\n",
            r#"{"reason":"compiler-message","message":{"level":"error","message":"real error","code":{"code":"E0001"},"spans":[{"file_name":"src/a.rs","line_start":1,"column_start":1,"is_primary":true}],"children":[]}}"#
        );
        let fixer = CompilerFixer::new(PathBuf::from("."));
        let diagnostics = fixer.parse_diagnostics(&mixed);
        assert_eq!(
            diagnostics.len(),
            1,
            "text fallback must be suppressed when JSON parsing yielded any diagnostics"
        );
        assert_eq!(diagnostics[0].message, "real error");
    }
}
