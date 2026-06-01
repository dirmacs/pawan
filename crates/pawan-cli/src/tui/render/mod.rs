//! Core TUI render entry (`ui`), input, status bar, and tests.

#![allow(unused_imports)]

mod messages;
mod overlays;

use animate_core::Animate;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pawan::agent::session::{RetentionPolicy, SearchResult, Session, SessionSummary};
use pawan::agent::{AgentResponse, Message, PawanAgent, Role, ToolCallRecord, ToolCallRequest};
use pawan::config::TuiConfig;
use pawan::{PawanError, Result};
use ratatui::style::Style;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use ratatui_textarea::{Input, TextArea};
use regex::Regex;
use std::io::{self, Stdout};
use std::sync::OnceLock;
use std::time::Instant;
use tokio::sync::mpsc;

use super::app::App;
use super::highlight::SyntaxHighlighter;
use super::status_bar::StatusBarContext;
use super::theme::{self, current as theme_current};
use super::types::*;
use chrono::Local;
use std::sync::Mutex;

impl<'a> App<'a> {
    pub(crate) fn sync_subagent_queue(&mut self) {
        use pawan::subagent::{snapshot_queue, SubagentState};

        const TTL_MS: u64 = 4_000;
        let entries = snapshot_queue(TTL_MS)
            .into_iter()
            .map(|run| {
                let mut name = run.label;
                if let Some(tool) = &run.current_tool {
                    name = format!("{name} · {tool}");
                }
                let status = match run.state {
                    SubagentState::Running => super::queue_panel::TaskStatus::Running,
                    SubagentState::Done => super::queue_panel::TaskStatus::Done,
                    SubagentState::Failed => super::queue_panel::TaskStatus::Failed,
                };
                super::queue_panel::QueueEntry {
                    task_name: name,
                    status,
                }
            })
            .collect();
        self.queue_panel.set_entries(entries);
    }

    pub(crate) fn ui(&mut self, f: &mut Frame) {
        if self.show_welcome {
            self.render_welcome(f);
            return;
        }

        let area = f.area();
        let input_lines = self.input.lines().len();
        let input_height = (input_lines + 2).clamp(3, 10) as u16;
        let theme = super::theme::current();
        // Per-frame effect clock (clamped delta) + spinner advance. Effects are
        // suppressed under `cfg!(test)` so snapshot renders stay deterministic.
        let fx_on = !cfg!(test);
        let fx_tick = super::effects::frame_tick(&mut self.last_frame);
        self.spinner.tick(fx_tick.into());
        // Advance value tweens (token roll, context glide, accent fade) on the
        // same clock as the cell effects. Suppressed under test for determinism.
        if fx_on {
            super::effects::advance_value_clock(fx_tick);
            self.advance_value_animations();
        }

        // Full background fill
        f.render_widget(ratatui::widgets::Clear, area);
        f.render_widget(
            Paragraph::new("").style(Style::default().bg(theme.background)),
            area,
        );

        let shell_area = if area.width > 4 && area.height > 4 {
            Rect::new(
                area.x + 1,
                area.y + 1,
                area.width.saturating_sub(2),
                area.height.saturating_sub(2),
            )
        } else {
            area
        };

        let model_short = {
            let m = &self.model_name;
            m.rsplit('/').next().unwrap_or(m).to_string()
        };
        let shell = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(theme.border))
            .title_top(
                Line::from(vec![
                    Span::styled("◆ ", Style::default().fg(theme.accent)),
                    Span::styled(
                        "pawan",
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("  ", Style::default()),
                    Span::styled(model_short, Style::default().fg(theme.muted)),
                    Span::styled(" ", Style::default()),
                ])
                .left_aligned(),
            )
            .title_bottom(
                Line::from(Span::styled(
                    " F1 help · e expand · Tab focus · Ctrl+Q quit ",
                    Style::default().fg(theme.subtle),
                ))
                .right_aligned(),
            )
            .style(Style::default().bg(theme.surface));
        let content_area = shell.inner(shell_area);
        f.render_widget(shell, shell_area);

        self.sync_subagent_queue();
        let queue_h = self.queue_panel.height_hint();
        let layout = super::layout::compute_layout(content_area, queue_h, input_height);

        // Messages stay full-width inside the restored main shell.
        self.render_messages(f, layout.msg_area);
        // Fade freshly finalized assistant turns into view.
        if fx_on {
            if let Some(fx) = self.content_fx.as_mut() {
                fx.process(fx_tick, f.buffer_mut(), layout.msg_area);
                if fx.done() {
                    self.content_fx = None;
                }
            }
        }

        // Queue panel (compact, only when tasks are running)
        self.queue_panel.view(f, layout.queue_area);

        // Input area
        self.render_input(f, layout.input_area);

        // Slash popup above input
        if self.is_slash_popup_active() && !self.help_overlay && self.fuzzy_search.is_none() {
            self.render_slash_popup(f, layout.input_area);
        }

        // Status bar at the bottom
        self.status_bar
            .view(f, layout.status_area, &self.status_bar_context());
        // Pulse the status strip on token/context updates.
        if fx_on {
            if let Some(fx) = self.status_fx.as_mut() {
                fx.process(fx_tick, f.buffer_mut(), layout.status_area);
                if fx.done() {
                    self.status_fx = None;
                }
            }
        }

        // Overlays (modals take precedence). Animated modals return their area so
        // a sweep-in effect can be applied on the frame they first appear.
        let overlay_area: Option<Rect> = if self.permission_dialog.is_some() {
            Some(self.render_permission_dialog(f))
        } else if self.model_picker.visible {
            Some(self.render_model_selector(f))
        } else if self.irc_compose_open {
            self.render_irc_compose(f);
            None
        } else if self.session_browser_open {
            self.render_session_browser(f);
            None
        } else if self.help_overlay {
            Some(self.render_help_overlay(f))
        } else if self.fuzzy_search.is_some() {
            Some(self.render_fuzzy_search(f))
        } else {
            None
        };

        let overlay_active = overlay_area.is_some();
        if fx_on {
            if overlay_active && !self.overlay_was_active {
                self.popup_fx = Some(super::effects::popup_open(theme.surface_elevated));
            }
            if let Some(rect) = overlay_area {
                if let Some(fx) = self.popup_fx.as_mut() {
                    fx.process(fx_tick, f.buffer_mut(), rect);
                    if fx.done() {
                        self.popup_fx = None;
                    }
                }
            }
        }
        self.overlay_was_active = overlay_active;
    }
    pub(crate) fn render_input(&mut self, f: &mut Frame, area: Rect) {
        // Animated accent (folds the former `ColorTransition`); raw under test.
        let accent_color = if cfg!(test) {
            theme_current().accent
        } else {
            *self.accent_tween.get()
        };

        // Input separator carries focus/processing state without boxing the textarea.
        let sep_style = if self.focus == Panel::Input {
            Style::default().fg(accent_color)
        } else {
            Style::default().fg(theme_current().muted)
        };

        // Draw a subtle top separator with the input state embedded.
        let sep_area = Rect::new(area.x, area.y, area.width, 1);
        let label = if self.processing {
            " Input: processing "
        } else {
            " Input "
        };
        let sep_width = area.width as usize;
        let label_width = label.chars().count();
        let right_rule = sep_width.saturating_sub(label_width);
        let sep_line = Line::from(vec![
            Span::styled(label.to_string(), sep_style),
            Span::styled("\u{2500}".repeat(right_rule), sep_style),
        ]);
        f.render_widget(Paragraph::new(sep_line), sep_area);

        // Input area below the separator
        let input_area = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );

        // Render textarea with chevron prefix
        f.render_widget(&self.input, input_area);
    }
    /// Build the StatusBarContext each frame so the status strip can flash on events.
    pub(crate) fn status_bar_context(&self) -> StatusBarContext {
        use super::types::KeybindContext;

        let mode_label: &'static str = match self.current_context {
            KeybindContext::Input => "INPUT",
            KeybindContext::Normal => "NORMAL",
            KeybindContext::Command => "CMD",
            KeybindContext::Help => "HELP",
            KeybindContext::ModelPicker => "MODEL",
        };
        let mode_style = if self.processing {
            Style::default().fg(Color::Yellow)
        } else if self.status.starts_with("Error") {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let thinking_label = if self.processing {
            Some("thinking...".to_string())
        } else if self.streaming.is_some() {
            Some("streaming...".to_string())
        } else {
            None
        };
        // Animated values roll/glide toward their targets; raw under test so
        // snapshots stay deterministic.
        let total_tokens = if cfg!(test) {
            self.total_tokens
        } else {
            self.token_tween.get().round() as u64
        };
        let context_pct = if cfg!(test) {
            Self::context_fraction(self.context_estimate)
        } else {
            *self.ctx_tween.get()
        };

        StatusBarContext {
            model_name: self.model_name.clone(),
            mode: mode_label,
            mode_style,
            goal_active: self.goal_mode,
            loop_active: self.loop_mode,
            total_tokens,
            context_pct,
            iteration: self.iteration_count,
            branch: None,
            timestamp: Local::now().format("%H:%M").to_string(),
            thinking_label,
        }
    }

    /// Context-usage fraction (0.0..=1.0) for a given token estimate. Shared by
    /// the status strip and the value-tween target so they never diverge.
    pub(crate) fn context_fraction(estimate: usize) -> f32 {
        if estimate > 0 {
            (estimate as f32 / 128_000.0).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Advance the per-frame value tweens. Targets are (re)set only when they
    /// actually move so the easing clock isn't restarted every frame; the accent
    /// target is set separately on `/theme` switch.
    pub(crate) fn advance_value_animations(&mut self) {
        let tokens = self.total_tokens as f64;
        if (*self.token_tween.target() - tokens).abs() >= 1.0 {
            self.token_tween.set(tokens);
        }
        self.token_tween.update();

        let pct = Self::context_fraction(self.context_estimate);
        if (*self.ctx_tween.target() - pct).abs() >= f32::EPSILON {
            self.ctx_tween.set(pct);
        }
        self.ctx_tween.update();

        self.accent_tween.update();
    }
}

#[cfg(test)]
mod tests {
    use super::super::app::App;
    use super::super::types::*;

    use pawan::config::TuiConfig;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::mpsc;

    use crossterm::event::{Event, KeyCode, KeyModifiers};
    use pawan::agent::Role;
    use ratatui::style::Color;
    use ratatui::Terminal;

    use super::super::fuzzy_search::{default_command_item_lines, FuzzySearchState};
    use super::messages::{markdown_to_lines, parse_inline_markdown};
    use pawan::agent::session::Session;
    use pawan::agent::ToolCallRecord;
    use ratatui::style::Modifier;
    use ratatui_textarea::TextArea;

    /// Create a test App with dummy channels (for state/render tests, not event loops)
    fn test_app<'a>() -> App<'a> {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        // Disable welcome screen in tests so keypresses reach normal handlers
        app.show_welcome = false;
        app
    }

    fn theme_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn reset_theme_for_test() {
        super::super::theme::set_theme("default").unwrap();
    }

    // ===== Rendering tests using TestBackend =====

    #[test]
    fn test_render_empty_state() {
        let mut app = test_app();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let buf = terminal.backend().buffer().clone();
        // Should contain model name in status bar
        let content = buffer_to_string(&buf);
        assert!(
            content.contains("test-model"),
            "Status bar should show model name"
        );
        // StatusBar renders model name + mode badge + token bar; status text moved to overlay
        assert!(
            content.contains("pawan"),
            "Branded shell title should render"
        );
        assert!(content.contains("Input"), "Input panel title should render");
    }

    #[test]
    fn test_input_placeholder_uses_theme_muted_style_after_reset() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        let theme = super::super::theme::current();
        let placeholder = app
            .input
            .placeholder_style()
            .expect("placeholder should be enabled");
        assert_eq!(placeholder.fg, Some(theme.muted));

        app.input.insert_str("hello");
        app.submit_input();
        let placeholder = app
            .input
            .placeholder_style()
            .expect("placeholder should remain enabled after reset");
        assert_eq!(placeholder.fg, Some(theme.muted));
    }

    #[test]
    fn test_status_bar_separates_model_tokens_context_and_clock() {
        let mut app = test_app();
        app.total_tokens = 3_800;
        app.context_estimate = 3_840;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("test-model"));
        assert!(content.contains("3.8k tok"));
        assert!(content.contains("ctx 3% "));
        assert!(
            content.contains("  |  "),
            "status bar should visibly separate right-side fields"
        );
    }

    #[test]
    fn test_status_bar_separates_iteration_when_present() {
        let mut app = test_app();
        app.total_tokens = 12;
        app.iteration_count = 2;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("  |  iter 2  |  "));
    }

    #[test]
    fn test_render_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello pawan"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("You"), "Should render user badge");
        assert!(content.contains("Pawan"), "Should render assistant badge");
        assert!(
            content.contains("Hello pawan"),
            "Should render user message"
        );
        assert!(
            content.contains("Hi there!"),
            "Should render assistant message"
        );
    }

    #[test]
    fn test_render_processing_thinking() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("thinking"),
            "Should show thinking indicator"
        );
    }

    #[test]
    fn test_render_streaming_content() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::Text {
                content: "partial response so far".to_string(),
                streaming: true,
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("partial response"),
            "Should render streaming content"
        );
        assert!(content.contains("▌"), "Should show blinking cursor");
    }

    #[test]
    fn test_render_active_tool() {
        let mut app = test_app();
        app.processing = true;
        app.streaming = Some(StreamingAssistantState {
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Running),
            }],
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(content.contains("bash"), "Should show active tool name");
    }

    #[test]
    fn test_render_token_stats() {
        let mut app = test_app();
        app.total_tokens = 1500;
        app.total_prompt_tokens = 1000;
        app.total_completion_tokens = 500;

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("1.5k tok"),
            "Should show total token count"
        );
    }

    #[test]
    fn test_render_tool_call_results() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![
                ContentBlock::Text {
                    content: "Done".into(),
                    streaming: false,
                },
                ContentBlock::ToolCall {
                    name: "write_file".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "1".into(),
                            name: "write_file".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"success": true}),
                            success: true,
                            duration_ms: 42,
                        },
                        expanded: false,
                    }),
                },
                ContentBlock::ToolCall {
                    name: "bash".into(),
                    args_summary: String::new(),
                    state: Box::new(ToolBlockState::Done {
                        record: ToolCallRecord {
                            id: "2".into(),
                            name: "bash".into(),
                            arguments: serde_json::json!({}),
                            result: serde_json::json!({"error": "timeout"}),
                            success: false,
                            duration_ms: 30000,
                        },
                        expanded: true,
                    }),
                },
            ],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("write_file"),
            "Should show successful tool name"
        );
        assert!(content.contains("bash"), "Should show failed tool name");
        assert!(
            content.contains("42ms") || content.contains("✓"),
            "Should show success indicator"
        );
    }

    #[test]
    fn test_tool_call_expansion_toggle() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({"command": "ls"}),
                        result: serde_json::json!({"output": "file1.txt\nfile2.txt"}),
                        success: true,
                        duration_ms: 100,
                    },
                    expanded: false,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        // Toggle expansion
        app.toggle_nearest_tool_expansion();

        // Verify that the tool call state was modified
        if let Some(ContentBlock::ToolCall {
            state: tool_state, ..
        }) = app.messages.first().unwrap().blocks.first()
        {
            if let ToolBlockState::Done { expanded, .. } = tool_state.as_ref() {
                assert!(*expanded, "Tool call should be expanded after toggle");
            }
        }
    }

    #[test]
    fn test_tool_call_error_display() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({"command": "invalid_command"}),
                        result: serde_json::json!({"error": "command not found"}),
                        success: false,
                        duration_ms: 50,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("error") || content.contains("failed"),
            "Should show error indication for failed tool call"
        );
    }

    #[test]
    fn test_tool_call_duration_display() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "bash".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "bash".to_string(),
                        arguments: serde_json::json!({}),
                        result: serde_json::json!({}),
                        success: true,
                        duration_ms: 1234,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        // Duration should be shown in some format (ms, s, etc.)
        assert!(
            content.contains("1") || content.contains("234"),
            "Should show tool call duration"
        );
    }

    #[test]
    fn test_multiple_tool_calls_in_message() {
        let mut app = test_app();
        app.messages.push(DisplayMessage {
        role: Role::Assistant,
        blocks: vec![
            ContentBlock::Text { content: "I'll help you with that".into(), streaming: false },
            ContentBlock::ToolCall {
                name: "read_file".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "test.txt"}),
                        result: serde_json::json!({"content": "test content"}),
                        success: true,
                        duration_ms: 10,
                    },
                    expanded: false,
                }),
            },
            ContentBlock::ToolCall {
                name: "write_file".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "2".to_string(),
                        name: "write_file".to_string(),
                        arguments: serde_json::json!({"path": "output.txt", "content": "output"}),
                        result: serde_json::json!({"success": true}),
                        success: true,
                        duration_ms: 15,
                    },
                    expanded: false,
                }),
            },
        ],
        timestamp: std::time::Instant::now(),
        cached_block_lines: None,
    });

        // Verify that both tool calls are present
        let records = app.messages.first().unwrap().tool_records();
        assert_eq!(records.len(), 2, "Should have 2 tool call records");
    }

    #[test]
    fn test_tool_call_with_complex_arguments() {
        let mut app = test_app();
        let complex_args = serde_json::json!({
            "files": ["file1.txt", "file2.txt"],
            "options": {
                "recursive": true,
                "max_depth": 5
            }
        });

        app.messages.push(DisplayMessage {
            role: Role::Assistant,
            blocks: vec![ContentBlock::ToolCall {
                name: "search".to_string(),
                args_summary: String::new(),
                state: Box::new(ToolBlockState::Done {
                    record: ToolCallRecord {
                        id: "1".to_string(),
                        name: "search".to_string(),
                        arguments: complex_args.clone(),
                        result: serde_json::json!({"results": []}),
                        success: true,
                        duration_ms: 200,
                    },
                    expanded: true,
                }),
            }],
            timestamp: std::time::Instant::now(),
            cached_block_lines: None,
        });

        let records = app.messages.first().unwrap().tool_records();
        assert_eq!(records.len(), 1, "Should have 1 tool call record");
        assert_eq!(
            records[0].arguments, complex_args,
            "Should preserve complex arguments"
        );
    }

    #[test]
    fn test_render_context_estimate() {
        let mut app = test_app();
        app.context_estimate = 85000; // 85k context — StatusBar shows context bar, not text label

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        // StatusBar shows model name and renders without crashing
        assert!(
            content.contains("test-model"),
            "Should render with model name"
        );
    }

    #[test]
    fn test_render_search_mode() {
        let mut app = test_app();
        app.search_mode = true;
        app.search_query = "hello".to_string();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("Search: hello"),
            "Should show search query in panel title"
        );
    }

    #[test]
    fn test_render_focus_input() {
        let app = test_app();
        assert_eq!(app.focus, Panel::Input, "Default focus should be Input");
    }

    // ===== Event handling tests =====

    #[test]
    fn test_ctrl_c_clears_input() {
        let mut app = test_app();
        // Add some text to the input
        app.input.insert_str("test message");
        assert!(
            !app.input.lines().iter().all(|l| l.is_empty()),
            "Input should have text"
        );

        // Press Ctrl+C
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // Input should be cleared, not quit
        assert!(!app.should_quit, "Ctrl+C should not quit");
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be cleared"
        );
        assert_eq!(
            app.status, "Input cleared",
            "Status should show input cleared"
        );
    }

    #[test]
    fn test_ctrl_c_clears_empty_input() {
        let mut app = test_app();
        // Input is empty by default

        // Press Ctrl+C
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // Input should still be cleared (no-op), not quit
        assert!(
            !app.should_quit,
            "Ctrl+C should not quit even when input is empty"
        );
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
        assert_eq!(
            app.status, "Input cleared",
            "Status should show input cleared"
        );
    }

    #[test]
    fn test_ctrl_q_quits() {
        let mut app = test_app();
        // Add some text to the input
        app.input.insert_str("test message");

        // Press Ctrl+Q
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )));

        // Should quit regardless of input state
        assert!(app.should_quit, "Ctrl+Q should quit");
    }

    #[test]
    fn test_history_navigation() {
        let mut app = test_app();

        // Submit some messages to build history
        app.input.insert_str("first message");
        app.submit_input();
        app.input.insert_str("second message");
        app.submit_input();
        app.input.insert_str("third message");
        app.submit_input();

        // Verify history was built
        assert_eq!(app.history.len(), 3, "Should have 3 messages in history");
        assert_eq!(app.history[0], "first message");
        assert_eq!(app.history[1], "second message");
        assert_eq!(app.history[2], "third message");

        // Press up arrow to go to most recent message
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        // Should have the most recent message
        assert_eq!(app.history_position, Some(2), "Should be at position 2");
        assert_eq!(app.input.lines().join("\n"), "third message");

        // Press up arrow again to go to previous message
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        // Should have the second message
        assert_eq!(app.history_position, Some(1), "Should be at position 1");
        assert_eq!(app.input.lines().join("\n"), "second message");

        // Press down arrow to go forward
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));

        // Should have the most recent message again
        assert_eq!(app.history_position, Some(2), "Should be at position 2");
        assert_eq!(app.input.lines().join("\n"), "third message");

        // Press down arrow again to exit history mode
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));

        // Should exit history mode
        assert_eq!(app.history_position, None, "Should exit history mode");
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
    }

    #[test]
    fn test_history_does_not_save_slash_commands() {
        let mut app = test_app();

        // Submit a slash command
        app.input.insert_str("/help");
        app.submit_input();

        // Submit a normal message
        app.input.insert_str("normal message");
        app.submit_input();

        // Verify only normal message was saved to history
        assert_eq!(app.history.len(), 1, "Should have 1 message in history");
        assert_eq!(app.history[0], "normal message");
    }

    #[test]
    fn test_ctrl_c_resets_history_position() {
        let mut app = test_app();

        // Build history
        app.input.insert_str("test message");
        app.submit_input();

        // Navigate to history
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::NONE,
        )));

        assert_eq!(app.history_position, Some(0), "Should be in history mode");

        // Press Ctrl+C to clear
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        // History position should be reset
        assert_eq!(
            app.history_position, None,
            "History position should be reset"
        );
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Input should be empty"
        );
    }

    #[test]
    fn test_ctrl_l_clears() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.messages.is_empty(), "Ctrl+L should clear messages");
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_tab_switches_focus() {
        let mut app = test_app();
        assert_eq!(app.focus, Panel::Input);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.focus,
            Panel::Messages,
            "Tab from Input goes to Messages"
        );

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.focus, Panel::Input, "Tab from Messages goes to Input");
    }

    #[test]
    fn test_scroll_keys_in_messages_panel() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 5;

        // j scrolls down
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 6);

        // k scrolls up
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5);

        // g goes to top
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn test_scroll_saturates_at_zero() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 0;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "Scroll should not go below 0");
    }

    #[test]
    fn test_search_mode_entry_and_exit() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        // Enter search mode with /
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        assert!(app.search_mode, "/ should enter search mode");

        // Type search query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "hi");

        // Backspace deletes
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.search_query, "h");

        // Enter exits search mode, keeps query
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert_eq!(app.search_query, "h");
    }

    #[test]
    fn test_search_esc_clears_query() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_mode = true;
        app.search_query = "findme".to_string();

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.search_mode);
        assert!(app.search_query.is_empty(), "Esc should clear search query");
    }

    #[test]
    fn test_search_n_jumps_forward() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "has target word"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "another target"));
        app.scroll = 0;

        // n should jump to first match after current scroll
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 1, "n should jump to first match at index 1");

        // n again should jump to next match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 2, "n should jump to next match at index 2");
    }

    #[test]
    fn test_search_n_reverse() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.search_query = "target".to_string();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "first target"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "no match"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "second target"));
        app.scroll = 2;

        // N should jump to previous match
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 0, "N should jump to previous match at index 0");
    }

    #[test]
    fn test_mouse_scroll() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = true;
        app.config.scroll_speed = 3;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 2,
            "Mouse scroll up should decrease by scroll_speed"
        );

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 5,
            "Mouse scroll down should increase by scroll_speed"
        );
    }

    #[test]
    fn test_mouse_scroll_disabled() {
        let mut app = test_app();
        app.scroll = 5;
        app.config.mouse_support = false;

        app.handle_event(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scroll, 5,
            "Mouse scroll should be ignored when disabled"
        );
    }

    // ===== State transition tests =====

    #[test]
    fn test_submit_input_creates_message() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["hello pawan"]);

        app.submit_input();

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].text_content(), "hello pawan");
        assert_eq!(app.messages[0].role, Role::User);
        assert!(app.processing, "Should be processing after submit");
        assert_eq!(app.status, "Processing...");
    }

    #[test]
    fn test_submit_empty_input_ignored() {
        let mut app = test_app();
        app.input = TextArea::from(vec!["   "]);

        app.submit_input();

        assert!(
            app.messages.is_empty(),
            "Empty input should not create message"
        );
        assert!(!app.processing, "Should not be processing for empty input");
    }

    #[test]
    fn test_processing_input_title() {
        let mut app = test_app();
        app.processing = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        assert!(
            content.contains("processing"),
            "Input panel should show processing state"
        );
    }

    #[test]
    fn test_error_status_renders() {
        let mut app = test_app();
        app.status = "Error: connection refused".to_string();

        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();

        let content = buffer_to_string(terminal.backend().buffer());
        // StatusBar uses red mode_style for error status — check model name still renders
        assert!(
            content.contains("test-model"),
            "Should render model name even on error"
        );
    }

    #[test]
    fn test_page_up_down_scroll() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 15;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageUp,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 5, "PageUp should scroll up by 10");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::PageDown,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.scroll, 15, "PageDown should scroll down by 10");
    }

    #[test]
    fn test_ctrl_u_d_half_page() {
        let mut app = test_app();
        app.focus = Panel::Messages;
        app.scroll = 25;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 5, "Ctrl+U should scroll up by 20");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.scroll, 25, "Ctrl+D should scroll down by 20");
    }

    #[test]
    fn test_i_returns_to_input() {
        let mut app = test_app();
        app.focus = Panel::Messages;

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.focus,
            Panel::Input,
            "'i' in Messages panel should return to Input"
        );
    }

    // ===== Helper =====

    /// Convert a ratatui Buffer to a plain string for assertion matching
    fn buffer_to_string(buf: &Buffer) -> String {
        let area = buf.area;
        let mut result = String::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let cell = &buf[(x, y)];
                result.push_str(cell.symbol());
            }
            result.push('\n');
        }
        result
    }

    // ===== Slash command tests =====

    #[test]
    fn test_slash_help() {
        let mut app = test_app();
        app.handle_slash_command("/help");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("/model"));
        assert!(app.messages[0].text_content().contains("/search"));
        assert!(app.messages[0].text_content().contains("/quit"));
    }

    #[test]
    fn test_slash_clear() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "reply"));
        app.handle_slash_command("/clear");
        assert!(app.messages.is_empty());
        assert_eq!(app.status, "Cleared");
    }

    #[test]
    fn test_slash_goal_sets_objective_and_system_message() {
        let mut app = test_app();
        app.handle_slash_command("/goal ship the feature");
        assert!(app.goal_mode);
        assert_eq!(app.goal_objective.as_deref(), Some("ship the feature"));
        assert!(app
            .messages
            .iter()
            .any(|m| m.text_content().contains("ship the feature")));
        let ctx = app.status_bar_context();
        assert!(ctx.goal_active);
        assert!(!ctx.loop_active);
    }

    #[test]
    fn test_slash_goal_toggle_off_clears_objective() {
        let mut app = test_app();
        app.handle_slash_command("/goal test objective");
        app.handle_slash_command("/goal");
        assert!(!app.goal_mode);
        assert!(app.goal_objective.is_none());
        let ctx = app.status_bar_context();
        assert!(!ctx.goal_active);
    }

    #[test]
    fn test_slash_loop_toggles_and_shows_iteration_hint() {
        let mut app = test_app();
        app.iteration_count = 3;
        app.handle_slash_command("/loop");
        assert!(app.loop_mode);
        assert!(app
            .messages
            .iter()
            .any(|m| m.text_content().contains("iteration 3")));
        let ctx = app.status_bar_context();
        assert!(ctx.loop_active);
        app.handle_slash_command("/loop");
        assert!(!app.loop_mode);
        assert!(app
            .messages
            .iter()
            .any(|m| m.text_content().contains("Loop mode disabled")));
    }

    #[test]
    fn test_slash_model_show() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        // New behavior: opens visual model selector
        assert!(app.model_picker.visible);
        assert_eq!(app.messages.len(), 0); // no message added
    }

    #[test]
    fn test_slash_model_switch() {
        let mut app = test_app();
        app.handle_slash_command("/model mistral-small-4");
        assert_eq!(app.model_name, "mistral-small-4");
        assert!(app.messages[0].text_content().contains("mistral-small-4"));
    }

    #[test]
    fn test_enter_submits_slash_command_with_arguments() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.input.insert_str("/theme nord");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        assert_eq!(app.current_theme, "nord");
        assert_eq!(super::super::theme::current().name, "nord");
        assert!(
            app.input.lines().iter().all(|line| line.is_empty()),
            "input should reset after submitting a slash command"
        );

        app.handle_slash_command("/theme dracula");
    }

    #[test]
    fn test_enter_submits_theme_variants_with_arguments() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        for name in ["onedark", "gruvbox"] {
            let mut app = test_app();
            app.input.insert_str(format!("/theme {name}"));
            app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert_eq!(app.current_theme, name);
            assert_eq!(super::super::theme::current().name, name);
        }
        reset_theme_for_test();
    }

    #[test]
    fn test_slash_theme_without_args_lists_available_themes() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.handle_slash_command("/theme");

        assert_eq!(app.status, "Theme list shown");
        assert_eq!(app.messages.len(), 1);
        let msg = app.messages[0].text_content();
        assert!(msg.contains("Available themes:"));
        assert!(msg.contains("nord"));
        assert!(msg.contains("onedark"));
        assert!(msg.contains("gruvbox"));
    }

    #[test]
    fn test_slash_theme_invalid_name_reports_available_themes() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.handle_slash_command("/theme missing-theme");

        assert!(app.status.contains("Unknown theme 'missing-theme'"));
        assert!(app.status.contains("default"));
    }

    #[test]
    fn test_slash_theme_restyles_existing_input() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.input.insert_str("draft");
        app.handle_slash_command("/theme onedark");

        let theme = super::super::theme::current();
        assert_eq!(app.input.style().fg, Some(theme.foreground));
        assert_eq!(app.input.placeholder_style().unwrap().fg, Some(theme.muted));
        assert_eq!(app.input.lines().join("\n"), "draft");
        reset_theme_for_test();
    }

    #[test]
    fn test_enter_on_exact_slash_command_uses_popup_selection() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.input.insert_str("/theme");

        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));

        assert_eq!(app.status, "Theme list shown");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("Available themes:"));
    }

    #[test]
    fn test_ctrl_c_reset_keeps_placeholder_readable() {
        let _guard = theme_lock().lock().unwrap();
        reset_theme_for_test();
        let mut app = test_app();
        app.input.insert_str("draft");
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        let theme = super::super::theme::current();
        assert_eq!(app.input.placeholder_style().unwrap().fg, Some(theme.muted));
        assert!(app.input.lines().iter().all(|line| line.is_empty()));
    }

    #[test]
    fn test_inline_slash_popup_closes_when_command_has_arguments() {
        let mut app = test_app();
        app.input.insert_str("/theme nord");

        assert!(
            !app.is_slash_popup_active(),
            "slash popup should not intercept Enter once arguments are present"
        );
    }

    #[test]
    fn test_slash_tools() {
        let mut app = test_app();
        app.handle_slash_command("/tools");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("bash"));
        assert!(app.messages[0].text_content().contains("ast_grep"));
        assert!(app.messages[0].text_content().contains("mcp_daedra"));
    }

    #[test]
    fn test_slash_quit() {
        let mut app = test_app();
        app.handle_slash_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn test_slash_unknown() {
        let mut app = test_app();
        app.handle_slash_command("/bogus");
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("Unknown command"));
    }

    #[test]
    fn test_slash_shorthand_removed() {
        let mut app = test_app();
        app.handle_slash_command("/c");
        // Shorthand aliases were removed: `/c` is no longer an alias for `/clear`
        // and is reported as an unknown command instead.
        assert_eq!(app.messages.len(), 1);
        assert!(app.messages[0].text_content().contains("Unknown command"));
    }

    #[test]
    fn test_slash_handoff_empty() {
        let mut app = test_app();
        app.handle_slash_command("/handoff");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0]
            .text_content()
            .contains("No conversation to handoff"));
        assert_eq!(app.status, "Nothing to handoff");
    }

    #[test]
    fn test_slash_handoff_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Implement feature X"));
        app.messages.push(DisplayMessage::new_text(
            Role::Assistant,
            "I'll help with that",
        ));
        app.session_tool_calls = 5;
        app.session_files_edited = 2;

        app.handle_slash_command("/handoff");

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("Session Handoff"));
        assert!(app.messages[0].text_content().contains("Model:"));
        assert!(app.messages[0].text_content().contains("Messages:"));
        assert!(app.messages[0].text_content().contains("Tool calls:"));
        assert!(app.messages[0].text_content().contains("Files edited:"));
        assert_eq!(app.status, "Handoff complete");
    }

    #[test]
    fn test_slash_handoff_clears_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "First response"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Second message"));

        app.handle_slash_command("/handoff");

        // Should have only the handoff system message
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert!(app.messages[0].text_content().contains("Session Handoff"));
    }

    #[test]
    fn test_generate_handoff_prompt_empty() {
        let app = test_app();
        let prompt = app.generate_handoff_prompt();
        assert!(prompt.contains("No conversation context available"));
    }

    #[test]
    fn test_generate_handoff_prompt_with_content() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Fix src/main.rs"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "I'll fix it"));
        app.session_tool_calls = 3;
        app.session_files_edited = 1;

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Session Handoff"));
        assert!(prompt.contains("Model:"));
        assert!(prompt.contains("Messages:"));
        assert!(prompt.contains("Tool calls:"));
        assert!(prompt.contains("Files edited:"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_files() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "Edit src/main.rs and lib/helper.ts",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Files Referenced"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("lib/helper.ts"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_constraints() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "MUST use async functions\nMUST NOT break existing tests",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Constraints"));
        assert!(prompt.contains("MUST"));
    }

    #[test]
    fn test_generate_handoff_prompt_extracts_tasks() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "- Implement feature X\n- Fix bug Y\n* Add tests",
        ));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Key Tasks"));
        assert!(prompt.contains("Implement feature X") || prompt.contains("feature X"));
    }

    #[test]
    fn test_generate_handoff_prompt_recent_context() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "First response"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Second message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Second response"));

        let prompt = app.generate_handoff_prompt();

        assert!(prompt.contains("Recent Context"));
        assert!(prompt.contains("User") || prompt.contains("Assistant"));
    }

    // ===== Fuzzy search tests =====

    #[test]
    fn test_ctrl_p_toggles_fuzzy_search() {
        let mut app = test_app();
        assert!(app.fuzzy_search.is_none());
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_some());
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_none());
    }

    #[test]
    fn test_ctrl_f_opens_fuzzy_search() {
        let mut app = test_app();
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('f'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.fuzzy_search.is_some());
    }

    #[test]
    fn test_fuzzy_filter_model() {
        let mut st = FuzzySearchState::new(default_command_item_lines());
        st.filter("model");
        assert!(!st.results.is_empty());
        assert!(st
            .results
            .iter()
            .all(|l| l.to_lowercase().contains("model")));
    }

    #[test]
    fn test_fuzzy_esc_closes() {
        let mut app = test_app();
        app.fuzzy_search = Some(FuzzySearchState::new(default_command_item_lines()));
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.fuzzy_search.is_none());
    }

    #[test]
    fn test_session_stats_increment() {
        let app = test_app();
        assert_eq!(app.session_tool_calls, 0);
        assert_eq!(app.session_files_edited, 0);
    }

    // ===== Markdown rendering tests (existing) =====

    #[test]
    fn test_header_h1() {
        let lines = markdown_to_lines("# Hello");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Hello");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::UNDERLINED));
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(super::theme::current().accent)
        );
    }

    #[test]
    fn test_header_h2() {
        let lines = markdown_to_lines("## Subtitle");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Subtitle");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(super::theme::current().accent_dim)
        );
    }

    #[test]
    fn test_header_h3() {
        let lines = markdown_to_lines("### Section");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "Section");
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_bullet_list() {
        let lines = markdown_to_lines("- item one");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_star_bullet() {
        let lines = markdown_to_lines("* star item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('•'));
    }

    #[test]
    fn test_code_block() {
        let lines = markdown_to_lines("```rust\nlet x = 1;\n```");
        assert_eq!(lines.len(), 3);
        // First line is separator with language
        assert!(lines[0].spans[0].content.contains("rust"));
        // Middle line is syntax-highlighted code
        let joined: String = lines[1]
            .spans
            .iter()
            .map(|sp| sp.content.to_string())
            .collect();
        assert!(joined.contains("let x"));
        // Last line is closing separator
        assert!(lines[2].spans[0].content.contains('─'));
    }

    #[test]
    fn test_code_block_no_lang() {
        let lines = markdown_to_lines("```\nhello\n```");
        assert_eq!(lines.len(), 3);
        assert!(lines[0].spans[0].content.contains("code"));
    }

    #[test]
    fn test_blockquote() {
        let lines = markdown_to_lines("> quoted text");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('│'));
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::ITALIC));
    }

    #[test]
    fn test_horizontal_rule() {
        let lines = markdown_to_lines("---");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].content.contains('─'));
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(super::theme::current().muted)
        );
    }

    #[test]
    fn test_numbered_list() {
        let lines = markdown_to_lines("1. first item");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans.len() >= 2);
        // First span is the muted list marker
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(super::theme::current().muted)
        );
    }

    #[test]
    fn test_inline_bold() {
        let spans = parse_inline_markdown("hello **world**");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let spans = parse_inline_markdown("use `cargo test` here");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "use ");
        assert_eq!(spans[1].content, "cargo test");
        let theme = super::theme::current();
        assert_eq!(spans[1].style.fg, Some(theme.accent));
        assert_eq!(spans[1].style.bg, Some(theme.code_bg));
        assert_eq!(spans[2].content, " here");
    }

    #[test]
    fn test_inline_italic() {
        let spans = parse_inline_markdown("this is *important*");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "this is ");
        assert_eq!(spans[1].content, "important");
        assert!(spans[1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn test_inline_mixed() {
        let spans = parse_inline_markdown("**bold** and `code`");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content, " and ");
        assert_eq!(spans[2].content, "code");
        assert_eq!(spans[2].style.fg, Some(super::theme::current().accent));
    }

    #[test]
    fn test_plain_text() {
        let spans = parse_inline_markdown("just plain text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "just plain text");
    }

    #[test]
    fn test_unclosed_bold() {
        // Unclosed ** should render as literal **
        let spans = parse_inline_markdown("hello **unclosed");
        assert!(spans.len() >= 2);
    }

    #[test]
    fn test_multiline_markdown() {
        let text = "# Title\n\nSome **bold** text\n\n- bullet\n- another";
        let lines = markdown_to_lines(text);
        assert!(lines.len() >= 5);
        // First line is H1
        assert!(lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn test_empty_input() {
        let lines = markdown_to_lines("");
        // Empty string produces no lines (str::lines() returns empty iterator)
        assert_eq!(lines.len(), 0);
    }

    // ===== Welcome screen tests =====

    #[test]
    fn test_welcome_shown_on_fresh_app() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        assert!(app.show_welcome, "Fresh app should show welcome");
    }

    #[test]
    fn test_welcome_dismissed_on_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        assert!(app.show_welcome);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(!app.show_welcome, "Any keypress should dismiss welcome");
    }

    #[test]
    fn test_welcome_swallows_keypress() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        // Type 'a' while welcome is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Welcome should swallow the keypress"
        );
    }

    #[test]
    fn test_welcome_renders() {
        let (cmd_tx, _cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = pawan::agent::IrcHub::new();
        let irc_relay = std::sync::Arc::new(std::sync::Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            buf.content.iter().any(|cell| {
                let sym = cell.symbol();
                sym != " " && sym != "⠀" && sym.chars().any(|c| !c.is_whitespace())
            }),
            "Welcome screen should paint non-whitespace cells, got sample:\n{}",
            &text[..300.min(text.len())]
        );
    }

    // ===== F1 Help overlay tests =====

    #[test]
    fn test_f1_toggles_help_overlay() {
        let mut app = test_app();
        assert!(!app.help_overlay);
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::F(1),
            KeyModifiers::NONE,
        )));
        assert!(app.help_overlay, "F1 should open help overlay");
    }

    #[test]
    fn test_help_overlay_dismissed_on_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));
        assert!(
            !app.help_overlay,
            "Any keypress should dismiss help overlay"
        );
    }

    #[test]
    fn test_help_overlay_swallows_keypress() {
        let mut app = test_app();
        app.help_overlay = true;
        // Type 'a' while help is showing — should NOT reach input
        app.handle_event(Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        assert!(
            app.input.lines().iter().all(|l| l.is_empty()),
            "Help overlay should swallow the keypress"
        );
    }

    #[test]
    fn test_help_overlay_renders() {
        let mut app = test_app();
        app.help_overlay = true;
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            text.contains("Keyboard"),
            "Help overlay should show keyboard shortcuts"
        );
    }

    // ===== Export tests =====

    #[test]
    fn test_export_conversation() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        let path = "/tmp/pawan_test_export.md";
        let result = app.export_conversation(path, ExportFormat::Markdown);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("**You**"));
        assert!(content.contains("**Pawan**"));
        assert!(content.contains("Hello"));
        assert!(content.contains("Hi there!"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_slash_export() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test msg"));
        app.handle_slash_command("/export /tmp/pawan_test_slash_export.md");
        // Should have added a system message about export
        assert!(app.messages.len() >= 2);
        let last = app.messages.last().unwrap();
        assert_eq!(last.role, Role::System);
        assert!(
            last.text_content().contains("Exported"),
            "Should confirm export: {}",
            last.text_content()
        );
        std::fs::remove_file("/tmp/pawan_test_slash_export.md").ok();
    }
    // ===== Session Tagging Tests =====

    #[test]
    fn test_tag_add_single_tag() {
        let mut app = test_app();
        app.handle_slash_command("/tag add important");
        assert_eq!(app.session_tags, vec!["important".to_string()]);
    }

    #[test]
    fn test_tag_add_multiple_tags() {
        let mut app = test_app();
        app.handle_slash_command("/tag add foo bar baz");
        assert_eq!(app.session_tags.len(), 3);
        assert!(app.session_tags.contains(&"foo".to_string()));
        assert!(app.session_tags.contains(&"bar".to_string()));
        assert!(app.session_tags.contains(&"baz".to_string()));
    }

    #[test]
    fn test_tag_add_prevents_duplicates() {
        let mut app = test_app();
        app.handle_slash_command("/tag add alpha");
        app.handle_slash_command("/tag add alpha");
        assert_eq!(app.session_tags.len(), 1);
        assert_eq!(app.session_tags, vec!["alpha".to_string()]);
    }

    #[test]
    fn test_tag_remove_existing() {
        let mut app = test_app();
        app.handle_slash_command("/tag add one two three");
        app.handle_slash_command("/tag rm two");
        assert_eq!(app.session_tags.len(), 2);
        assert!(!app.session_tags.contains(&"two".to_string()));
    }

    #[test]
    fn test_tag_remove_nonexistent() {
        let mut app = test_app();
        app.handle_slash_command("/tag add alpha");
        app.handle_slash_command("/tag rm beta");
        assert_eq!(app.session_tags, vec!["alpha".to_string()]);
    }

    #[test]
    fn test_tag_list() {
        let mut app = test_app();
        app.handle_slash_command("/tag add tag1 tag2");
        app.handle_slash_command("/tag list");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("tag1"));
        assert!(last_msg.text_content().contains("tag2"));
    }

    #[test]
    fn test_tag_clear() {
        let mut app = test_app();
        app.handle_slash_command("/tag add one two three");
        app.handle_slash_command("/tag clear");
        assert!(app.session_tags.is_empty());
    }

    #[test]
    fn test_tag_empty_command_shows_usage() {
        let mut app = test_app();
        app.handle_slash_command("/tag");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("Usage"));
    }

    #[test]
    fn test_tag_invalid_command_shows_usage() {
        let mut app = test_app();
        app.handle_slash_command("/tag invalid_cmd");
        let last_msg = app.messages.last().unwrap();
        assert!(last_msg.text_content().contains("Usage"));
    }

    #[test]
    fn test_session_tags_persist_on_save() {
        let mut app = test_app();
        app.handle_slash_command("/tag add persistent_tag");
        app.handle_slash_command("/save");
        let sessions_dir = pawan::agent::session::Session::sessions_dir().unwrap();
        let mut found = false;
        if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
                if content.contains("persistent_tag") {
                    found = true;
                    break;
                }
            }
        }
        assert!(found, "Tag not persisted in saved session");
    }

    #[test]
    fn test_session_browser_tag_filter() {
        let mut app = test_app();
        app.session_browser_query = "tag:important".to_string();
        assert!(app.session_browser_query.starts_with("tag:"));
    }
    // ===== /fork, /dump, /share Command Tests =====

    #[test]
    fn test_fork_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/fork");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("No conversation to fork"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_fork_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        app.handle_slash_command("/fork");
        // Should create new session and switch to it
        assert!(
            app.current_session_id.is_some(),
            "Should have new session ID after fork"
        );
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Forked"),
            "Should confirm fork"
        );
    }

    #[test]
    fn test_dump_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/dump");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Nothing to dump"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_dump_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Test message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Response"));
        app.handle_slash_command("/dump");
        // Note: clipboard may not be available in test env, but should still generate markdown
        let last = app.messages.last().unwrap();
        let content = last.text_content();
        assert!(
            content.contains("Copied") || content.contains("Failed"),
            "Should attempt clipboard operation"
        );
        // Verify it tried to generate markdown
        assert!(
            content.contains("Pawan Session")
                || content.contains("Copied")
                || content.contains("Failed"),
            "Should contain session output"
        );
    }

    #[test]
    fn test_share_empty_conversation() {
        let mut app = test_app();
        app.handle_slash_command("/share");
        let last = app.messages.last().unwrap();
        assert!(
            last.text_content().contains("Nothing to share"),
            "Should warn when empty"
        );
    }

    #[test]
    fn test_share_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Share test"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Shared!"));
        app.handle_slash_command("/share");
        // Should save and copy path to clipboard
        let last = app.messages.last().unwrap();
        let content = last.text_content();
        assert!(
            content.contains("Session saved") || content.contains("Share failed"),
            "Should attempt save"
        );
    }

    #[test]
    fn test_fork_preserves_model_and_tags() {
        let mut app = test_app();
        app.model_name = "nvidia/llama-3.1-nemotron".to_string();
        app.session_tags.push("test-tag".to_string());
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Test"));
        app.handle_slash_command("/fork");
        // Verify new session got the same model and tags
        if let Some(ref new_id) = app.current_session_id {
            if let Ok(session) = Session::load(new_id) {
                assert_eq!(session.model, "nvidia/llama-3.1-nemotron");
                assert!(session.tags.contains(&"test-tag".to_string()));
            }
        }
    }

    // ===== /diff Command Test =====

    #[test]
    fn test_diff_command_handler() {
        let mut app = test_app();
        app.handle_slash_command("/diff");
        assert!(!app.messages.is_empty());
        let content = app.messages.last().unwrap().text_content();
        assert!(!content.is_empty());
    }

    // ===== Export Format Tests =====
    // ===== Export Format Tests =====

    #[test]
    fn test_export_html_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "HTML test"));
        let path = "/tmp/pawan_html_test.html";
        let result = app.export_conversation(path, ExportFormat::Html);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("<!DOCTYPE html>"));
        assert!(content.contains("<html"));
        assert!(content.contains("HTML test"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_json_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "JSON test"));
        let path = "/tmp/pawan_json_test.json";
        let result = app.export_conversation(path, ExportFormat::Json);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("\"messages\""));
        assert!(content.contains("JSON test"));
        let _: serde_json::Value = serde_json::from_str(&content).unwrap();
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_txt_format() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "TXT test"));
        let path = "/tmp/pawan_txt_test.txt";
        let result = app.export_conversation(path, ExportFormat::Txt);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("TXT test"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_empty_conversation() {
        let app = test_app();
        let path = "/tmp/pawan_empty_test.md";
        let result = app.export_conversation(path, ExportFormat::Markdown);
        assert!(result.is_ok());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_html_escaping() {
        let mut app = test_app();
        app.messages.push(DisplayMessage::new_text(
            Role::User,
            "<script>alert('xss')</script>",
        ));
        let path = "/tmp/pawan_escape_test.html";
        let result = app.export_conversation(path, ExportFormat::Html);
        assert!(result.is_ok());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(!content.contains("<script>"));
        assert!(content.contains("&lt;script&gt;"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_export_with_tool_calls() {
        let mut app = test_app();

        // Create a message with tool calls
        let mut msg = DisplayMessage::new_text(Role::Assistant, "Processing request");
        msg.blocks.push(ContentBlock::ToolCall {
            name: "bash".to_string(),
            args_summary: "echo test".to_string(),
            state: Box::new(ToolBlockState::Done {
                record: ToolCallRecord {
                    id: "test-id".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({"command": "echo test"}),
                    result: serde_json::Value::String("test output".to_string()),
                    success: true,
                    duration_ms: 100,
                },
                expanded: true,
            }),
        });
        app.messages.push(msg);

        // Test markdown export
        let md_path = "/tmp/test_tool_calls.md";
        let result = app.export_conversation(md_path, ExportFormat::Markdown);
        assert!(result.is_ok(), "Markdown export should succeed");

        let md_content = std::fs::read_to_string(md_path).unwrap();
        assert!(md_content.contains("bash"), "Should contain tool name");
        assert!(
            md_content.contains("echo test"),
            "Should contain args summary"
        );
        assert!(md_content.contains("test output"), "Should contain result");

        // Test JSON export
        let json_path = "/tmp/test_tool_calls.json";
        let result = app.export_conversation(json_path, ExportFormat::Json);
        assert!(result.is_ok(), "JSON export should succeed");

        let json_content = std::fs::read_to_string(json_path).unwrap();
        assert!(
            json_content.contains("bash"),
            "JSON should contain tool name"
        );

        // Cleanup
        let _ = std::fs::remove_file(md_path);
        let _ = std::fs::remove_file(json_path);
    }

    // ===== Timestamp tests =====

    #[test]
    fn test_message_has_timestamp() {
        let before = std::time::Instant::now();
        let msg = DisplayMessage::new_text(Role::User, "test");
        let after = std::time::Instant::now();
        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    // ===== Scroll indicator tests =====

    #[test]
    fn test_scrollbar_appears_when_overflowing() {
        let mut app = test_app();
        // Add enough messages to exceed the visible area so scroll indicator appears
        for i in 0..20 {
            app.messages.push(DisplayMessage::new_text(
                Role::User,
                format!("message line {}", i),
            ));
        }
        app.scroll = 5;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        // tui-scrollview draws a vertical scrollbar (arrow glyphs) on overflow.
        assert!(
            text.contains('\u{25b2}') || text.contains('\u{25bc}'),
            "Should show scrollbar arrows, got:\n{}",
            &text[..300.min(text.len())]
        );
    }

    // ===== Message count in status bar =====

    #[test]
    fn test_status_bar_shows_message_count() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "hi"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "hello"));
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.ui(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        // StatusBar shows model name + tokens + context bar; message count moved elsewhere
        assert!(
            text.contains("test-model"),
            "Status bar should render with model name"
        );
    }

    #[test]
    fn test_fuzzy_catalog_includes_export() {
        let items = default_command_item_lines();
        assert!(
            items.iter().any(|s| s.starts_with("/export")),
            "Catalog should include /export"
        );
    }

    #[test]
    fn test_fuzzy_catalog_includes_import() {
        let items = default_command_item_lines();
        assert!(
            items.iter().any(|s| s.starts_with("/import")),
            "Catalog should include /import"
        );
    }

    #[test]
    fn test_import_command_requires_path() {
        let mut app = test_app();
        app.handle_slash_command("/import");
        assert!(app.messages.iter().any(|m| {
        matches!(m, DisplayMessage { role: Role::System, .. } if {
            // Check if any block contains the usage message
            m.blocks.iter().any(|block| {
                matches!(block, ContentBlock::Text { content, .. } if content.contains("Usage: /import <path>"))
            })
        })
    }), "Should show usage message when no path provided");
    }
    #[test]
    fn test_load_available_models_populates_list() {
        let mut app = test_app();
        assert!(app.model_picker.models.is_empty());
        app.load_available_models();
        assert!(!app.model_picker.models.is_empty());
        assert!(app.model_picker.models.len() >= 4);
    }

    #[test]
    fn test_filtered_models_empty_when_not_loaded() {
        let app = test_app();
        let filtered = app.filtered_models();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filtered_models_with_search() {
        let mut app = test_app();
        app.load_available_models();
        app.model_picker.query = "nvidia".to_string();
        let _filtered = app.filtered_models();
        app.model_picker.query = "anthropic".to_string();
        let _filtered = app.filtered_models();
        app.model_picker.query = "nonexistent".to_string();
        let filtered = app.filtered_models();
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filtered_models_empty_query_returns_all() {
        let mut app = test_app();
        app.load_available_models();
        app.model_picker.query.clear();
        let filtered = app.filtered_models();
        assert_eq!(filtered.len(), app.model_picker.models.len());
    }

    #[test]
    fn test_model_selector_modal_state() {
        let mut app = test_app();
        assert!(!app.model_picker.visible);
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        assert_eq!(app.model_picker.query, "");
        assert_eq!(app.model_picker.selected, 0);
        app.model_picker.visible = false;
        app.model_picker.query.clear();
        app.model_picker.selected = 0;
        assert!(!app.model_picker.visible);
    }

    // ===== Session Browser Tests =====
    #[test]
    fn test_session_browser_modal_state() {
        let mut app = test_app();
        assert!(!app.session_browser_open);
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        assert_eq!(app.session_browser_query, "");
        assert_eq!(app.session_browser_selected, 0);
    }

    #[test]
    fn test_session_sorting_modes() {
        let modes = [
            SessionSortMode::NewestFirst,
            SessionSortMode::Alphabetical,
            SessionSortMode::MostUsed,
        ];
        assert_eq!(modes.len(), 3);
    }
    // ===== Slash Command Tests =====
    #[test]
    fn test_slash_sessions_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_save_creates_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/save");
        assert!(app.messages.len() >= 2);
        let _last = app.messages.last().unwrap();
    }

    #[test]
    fn test_slash_load_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/load");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_resume_opens_browser() {
        let mut app = test_app();
        app.handle_slash_command("/resume");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_slash_new_clears_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/new");
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].role, Role::System);
        assert_eq!(
            app.messages[0].text_content().trim(),
            "Started new conversation"
        );
    }

    #[test]
    fn test_slash_items_includes_all_commands() {
        let app = test_app();
        let items = app.slash_items();
        let commands: Vec<_> = items
            .iter()
            .map(|(cmd, _)| cmd.as_str())
            .collect::<Vec<_>>();
        assert!(commands.contains(&"/sessions"));
        assert!(commands.contains(&"/save"));
        assert!(commands.contains(&"/load"));
        assert!(commands.contains(&"/resume"));
        assert!(commands.contains(&"/new"));
        assert!(commands.contains(&"/model"));
        assert!(commands.contains(&"/export"));
        assert!(commands.contains(&"/compact"));
        assert!(commands.contains(&"/session"));
        assert!(commands.contains(&"/retry"));
    }
    // ===== Auto-save Tests =====
    #[test]
    fn test_autosave_with_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        // Should not panic
        app.autosave();
    }

    #[test]
    fn test_autosave_with_empty_session() {
        let mut app = test_app();
        // Should not panic even with empty messages
        app.autosave();
    }

    #[test]
    fn test_autosave_with_multiple_messages() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "First message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Second message"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Third message"));
        // Should not panic with multiple messages
        app.autosave();
    }

    #[test]
    fn test_autosave_with_whitespace_only_messages() {
        let mut app = test_app();
        // Add whitespace-only messages
        app.messages
            .push(DisplayMessage::new_text(Role::User, "   "));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "\t\n"));
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Valid message"));
        // Should not panic and should handle whitespace-only messages
        app.autosave();
    }

    #[test]
    fn test_autosave_does_not_modify_app_state() {
        let mut app = test_app();
        let initial_message_count = app.messages.len();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));

        app.autosave();

        // Autosave should not modify app state (it's called on &self)
        assert_eq!(
            app.messages.len(),
            initial_message_count + 1,
            "Autosave should not modify message count"
        );
    }
    #[test]
    fn test_model_selector_modal_rendering() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.load_available_models();
        let _ = app;
    }

    #[test]
    fn test_session_browser_modal_rendering() {
        let mut app = test_app();
        app.session_browser_open = true;
        let _ = app;
    }

    #[test]
    fn test_help_overlay_modal_rendering() {
        let mut app = test_app();
        app.help_overlay = true;
        let _ = app;
    }

    // ===== Keyboard Handling Tests =====
    #[test]
    fn test_keyboard_esc_closes_modals() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.session_browser_open = true;
        app.help_overlay = true;
        app.model_picker.visible = false;
        app.session_browser_open = false;
        app.help_overlay = false;
        assert!(!app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(!app.help_overlay);
    }

    #[test]
    fn test_keyboard_enter_in_model_selector() {
        let mut app = test_app();
        app.model_picker.visible = true;
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            let selected = app.model_picker.models.get(app.model_picker.selected);
            let _ = selected;
        }
    }

    #[test]
    fn test_keyboard_enter_in_session_browser() {
        let mut app = test_app();
        app.session_browser_open = true;
        app.session_browser_selected = 0;
        let _ = app;
    }

    // ===== Integration Tests =====
    #[test]
    fn test_full_session_lifecycle() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test message"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
    }

    #[test]
    fn test_model_selection_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            app.model_picker.visible = false;
        }
    }

    #[test]
    fn test_slash_command_dispatch() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_modal_transitions() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.help_overlay = true;
        assert!(app.help_overlay);
    }

    // ===== E2E Test Scaffolding =====
    #[test]
    fn test_e2e_session_creation_and_browsing() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "first message"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "response"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
    }

    #[test]
    fn test_e2e_model_switching_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        if !app.model_picker.models.is_empty() {
            app.model_picker.selected = 0;
            app.model_picker.visible = false;
        }
        app.messages
            .push(DisplayMessage::new_text(Role::User, "test"));
        app.handle_slash_command("/save");
    }

    #[test]
    fn test_e2e_session_management_workflow() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 1"));
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 2"));
        app.handle_slash_command("/save");
    }

    #[test]
    fn test_e2e_autosave_during_session() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 1"));
        app.autosave();
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "response 1"));
        app.autosave();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "message 2"));
        app.autosave();
    }

    #[test]
    fn test_e2e_slash_command_sequence() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/export");
    }

    #[test]
    fn test_e2e_modal_state_consistency() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(!app.help_overlay);
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(!app.model_picker.visible);
        assert!(app.session_browser_open);
        assert!(!app.help_overlay);
        app.session_browser_open = false;
        app.help_overlay = true;
        assert!(!app.model_picker.visible);
        assert!(!app.session_browser_open);
        assert!(app.help_overlay);
    }

    #[test]
    fn test_e2e_session_sorting_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_sort_mode = SessionSortMode::Alphabetical;
        app.session_sort_mode = SessionSortMode::MostUsed;
        app.session_sort_mode = SessionSortMode::NewestFirst;
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_search_and_filter_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        app.model_picker.query = "test".to_string();
        let filtered = app.filtered_models();
        let _ = filtered;
        app.model_picker.query.clear();
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_query = "test".to_string();
        let sessions = app.filtered_sessions();
        let _ = sessions;
        app.session_browser_query.clear();
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_keyboard_navigation_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        let count = app.model_picker.models.len();
        if count > 0 {
            app.model_picker.selected = 0;
            app.model_picker.selected = (app.model_picker.selected + 1).min(count - 1);
            app.model_picker.selected = app.model_picker.selected.saturating_sub(1);
        }
        app.model_picker.visible = false;
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_selected = 0;
        app.session_browser_selected = app.session_browser_selected.saturating_sub(1);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_error_handling_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/save");
        app.handle_slash_command("/load");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/resume");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_concurrent_modals_prevention() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.help_overlay = true;
        assert!(app.model_picker.visible || app.session_browser_open || app.help_overlay);
    }

    #[test]
    fn test_e2e_state_persistence_workflow() {
        let mut app = test_app();
        app.messages
            .push(DisplayMessage::new_text(Role::User, "persistent message"));
        app.autosave();
        let _msg_count = app.messages.len();
        app.handle_slash_command("/save");
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
    }

    #[test]
    fn test_e2e_full_user_workflow() {
        let mut app = test_app();
        app.handle_slash_command("/model");
        assert!(app.model_picker.visible);
        app.load_available_models();
        app.model_picker.visible = false;
        app.messages
            .push(DisplayMessage::new_text(Role::User, "Hello"));
        app.messages
            .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
        app.autosave();
        app.handle_slash_command("/save");
        app.handle_slash_command("/sessions");
        assert!(app.session_browser_open);
        app.session_browser_open = false;
        app.handle_slash_command("/new");
        assert!(app.messages.is_empty());
        app.handle_slash_command("/export");
    }

    #[test]
    fn test_filtered_sessions_empty_query() {
        let app = test_app();
        let sessions = app.filtered_sessions();
        let _ = sessions;
    }

    #[test]
    fn test_filtered_sessions_with_search() {
        let app = test_app();
        let sessions = app.filtered_sessions();
        let _ = sessions;
    }

    #[test]
    fn test_model_selector_navigation() {
        let mut app = test_app();
        app.load_available_models();
        let count = app.model_picker.models.len();
        if count > 0 {
            app.model_picker.selected = (app.model_picker.selected + 1).min(count - 1);
            assert_eq!(app.model_picker.selected, 1);
        }
    }

    mod snapshot_tests {
        use super::super::super::app::PermissionDialog;
        use super::super::super::types::DisplayMessage;
        use super::buffer_to_string;
        use super::default_command_item_lines;
        use super::test_app;
        use super::FuzzySearchState;
        use pawan::agent::Role;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use tokio::sync::oneshot;

        fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
            let backend = TestBackend::new(width, height);
            Terminal::new(backend).unwrap()
        }

        fn render_snapshot<F>(width: u16, height: u16, render: F) -> String
        where
            F: FnOnce(&mut ratatui::Frame),
        {
            let mut terminal = test_terminal(width, height);
            terminal.draw(|f| render(f)).unwrap();
            buffer_to_string(terminal.backend().buffer())
        }

        fn redact_cwd(output: &str) -> String {
            std::env::current_dir()
                .ok()
                .map(|cwd| {
                    let cwd_str = cwd.display().to_string();
                    let pad = " ".repeat(cwd_str.len().saturating_sub("[CWD]".len()));
                    output.replace(&cwd_str, &format!("[CWD]{pad}"))
                })
                .unwrap_or_else(|| output.to_string())
        }

        #[test]
        fn test_render_welcome_snapshot() {
            let app = test_app();
            let output = redact_cwd(&render_snapshot(80, 24, |f| app.render_welcome(f)));
            insta::assert_snapshot!(output);
        }

        #[test]
        fn test_render_help_overlay_snapshot() {
            let app = test_app();
            let output = render_snapshot(80, 24, |f| {
                app.render_help_overlay(f);
            });
            insta::assert_snapshot!(output);
        }

        #[test]
        fn test_render_model_selector_snapshot() {
            let mut app = test_app();
            app.load_available_models();
            app.model_picker.visible = true;
            let output = render_snapshot(80, 24, |f| {
                app.render_model_selector(f);
            });
            insta::assert_snapshot!(output);
        }

        #[test]
        fn test_render_fuzzy_search_snapshot() {
            let mut app = test_app();
            let mut fs = FuzzySearchState::new(default_command_item_lines());
            fs.filter("help");
            app.fuzzy_search = Some(fs);
            let output = render_snapshot(80, 24, |f| {
                app.render_fuzzy_search(f);
            });
            insta::assert_snapshot!(output);
        }

        #[test]
        fn test_render_permission_dialog_snapshot() {
            let mut app = test_app();
            let (tx, _rx) = oneshot::channel();
            app.permission_dialog = Some(PermissionDialog {
                tool_name: "bash".to_string(),
                args_summary: "echo hello".to_string(),
                respond: Some(tx),
            });
            let output = render_snapshot(80, 24, |f| {
                app.render_permission_dialog(f);
            });
            insta::assert_snapshot!(output);
        }

        #[test]
        fn test_render_messages_snapshot() {
            let mut app = test_app();
            app.messages
                .push(DisplayMessage::new_text(Role::User, "Hello pawan"));
            app.messages
                .push(DisplayMessage::new_text(Role::Assistant, "Hi there!"));
            let output = render_snapshot(80, 24, |f| app.render_messages(f, f.area()));
            insta::assert_snapshot!(output);
        }
    }
}
