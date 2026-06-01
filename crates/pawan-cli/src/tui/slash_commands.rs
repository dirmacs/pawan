//! Slash command entrypoints and fuzzy catalog helpers.

use pawan::agent::Role;
use pawan::{PawanError, Result};

use super::app::{App, SlashCommandRegistry};
use super::types::*;

impl<'a> App<'a> {
    /// Shared entry used by all registered `SlashCommand.handler` pointers
    pub fn universal_slash_entry(app: &mut App<'_>, args: &[&str]) -> Result<()> {
        let c: String = app
            .slash_inflight
            .as_deref()
            .ok_or_else(|| PawanError::Agent("internal: missing slash context".to_string()))?
            .to_string();
        let a = args.first().copied().unwrap_or("");
        app.slash_route(&c, a);
        Ok(())
    }

    /// Handle slash commands locally without sending to the agent
    pub(crate) fn handle_slash_command(&mut self, cmd: &str) {
        let s = cmd.trim();
        let normalized: String = if let Some(rest) = s.strip_prefix(':') {
            if rest.is_empty() {
                "/".to_string()
            } else {
                format!("/{rest}")
            }
        } else {
            s.to_string()
        };
        let parts: Vec<&str> = normalized.splitn(2, ' ').collect();
        let command = parts[0];
        let arg = parts.get(1).map(|x| x.trim()).unwrap_or("");

        if self.slash_registry.get(command).is_none() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!(
                    "Unknown command: {}. Type /help for available commands.",
                    command
                ),
            ));
            return;
        }

        if let Some(sc) = self.slash_registry.get(command) {
            self.slash_inflight = Some(sc.name.clone());
            let a: [&str; 1] = [arg];
            let sargs: &[&str] = if arg.is_empty() { &[] } else { &a };
            let res = (sc.handler)(self, sargs);
            self.slash_inflight = None;
            if let Err(e) = res {
                self.messages
                    .push(DisplayMessage::new_text(Role::System, e.to_string()));
            }
        }
    }

    pub(crate) fn slash_route(&mut self, command: &str, arg: &str) {
        if self.slash_route_session(command, arg)
            || self.slash_route_model(command, arg)
            || self.slash_route_agent(command, arg)
            || self.slash_route_misc(command, arg)
        {
            return;
        }
        debug_assert!(
            false,
            "unregistered slash command in slash_route match: {command}"
        );
    }

    fn slash_route_session(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "/handoff" => self.slash_handoff(),
            "/fork" => self.slash_fork(),
            "/dump" => self.slash_dump(),
            "/save" => self.slash_save(),
            "/share" => self.slash_share(),
            "/sessions" => self.slash_sessions(),
            "/searchsessions" => self.slash_search_sessions(arg),
            "/prune" => self.slash_prune(arg),
            "/tag" => self.slash_tag(arg),
            "/load" => self.slash_load(arg),
            "/resume" => self.slash_resume(arg),
            "/new" => self.slash_new(),
            "/session" => self.slash_session(arg),
            _ => return false,
        }
        true
    }

    fn slash_route_model(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "/model" => self.slash_model(arg),
            "/tools" => self.slash_tools(),
            "/theme" => self.slash_theme(arg),
            "/compact" => self.slash_compact(arg),
            _ => return false,
        }
        true
    }

    fn slash_route_agent(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "/search" => self.slash_search(arg),
            "/heal" => self.slash_heal(),
            "/retry" => self.slash_retry(),
            "/loop" => self.apply_loop_command(),
            "/goal" => self.apply_goal_command(arg),
            "/irc" => self.slash_irc(),
            "/orchestrate" => self.apply_orchestrate_command(arg.trim()),
            _ => return false,
        }
        true
    }

    fn slash_route_misc(&mut self, command: &str, arg: &str) -> bool {
        match command {
            "/clear" => self.slash_clear(),
            "/quit" | "/exit" => self.slash_quit(),
            "/export" => self.slash_export(arg),
            "/diff" => self.slash_diff(arg),
            "/import" => self.slash_import(arg),
            "/help" => self.slash_help(),
            _ => return false,
        }
        true
    }
}

pub(crate) fn default_slash_fuzzy_lines() -> Vec<String> {
    let r = SlashCommandRegistry::built_in();
    let mut out: Vec<String> = r
        .all()
        .iter()
        .map(|c| format!("{} — {}", c.name, c.description))
        .collect();
    out.sort();
    out.extend(
        [
            (
                "/model qwen/qwen3.5-122b-a10b",
                "Qwen 3.5 122B (S tier, fast)",
            ),
            ("/model minimaxai/minimax-m2.5", "MiniMax M2.5 (SWE 80.2%)"),
            (
                "/model stepfun-ai/step-3.5-flash",
                "Step 3.5 Flash (S+ tier)",
            ),
            (
                "/model mistralai/mistral-small-4-119b-2603",
                "Mistral Small 4 119B",
            ),
        ]
        .into_iter()
        .map(|(c, d)| format!("{c} — {d}")),
    );
    out
}
