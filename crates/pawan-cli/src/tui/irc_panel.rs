//! IRC compose modal and inbox polling for orchestrator ↔ subagent messaging.

use pawan::agent::IrcMessage;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::app::App;
use super::types::{AgentCommand, DisplayMessage};
use pawan::agent::Role;

impl<'a> App<'a> {
    /// Drain the main agent inbox and surface messages in the transcript.
    pub(crate) fn poll_irc_inbox(&mut self) {
        let mut relay = self.irc_relay.lock().expect("irc relay lock");
        while let Some(msg) = relay.try_receive() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format_irc_line(&msg),
            ));
            self.status = format!("IRC from {}", msg.from);
        }
    }

    /// Parse `irc_compose_input` as `<peer> <message>` and dispatch via cmd_tx.
    pub(crate) fn submit_irc_compose(&mut self) {
        let line = self.irc_compose_input.trim();
        if line.is_empty() {
            self.irc_compose_open = false;
            self.irc_compose_input.clear();
            return;
        }

        let Some((to, body)) = line.split_once(char::is_whitespace) else {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "IRC usage: <peer> <message>  (peers: use /irc with no args to list)".to_string(),
            ));
            return;
        };
        let body = body.trim();
        if body.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "IRC usage: <peer> <message>".to_string(),
            ));
            return;
        }

        let to = to.trim().to_string();
        let body = body.to_string();
        let _ = self.cmd_tx.send(AgentCommand::IrcSend {
            to: to.clone(),
            body: body.clone(),
        });
        self.irc_compose_open = false;
        self.irc_compose_input.clear();
        self.status = format!("IRC → {to}");
        self.messages.push(DisplayMessage::new_text(
            Role::System,
            format!("IRC queued to {to}: {body}"),
        ));
    }

    pub(crate) fn open_irc_compose(&mut self) {
        self.irc_compose_open = true;
        self.irc_compose_input.clear();
        let peers = self.irc_relay.lock().expect("irc relay lock").list_peers();
        if peers.is_empty() {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                "IRC: no other agents registered yet. Compose as: <peer-id> <message>".to_string(),
            ));
        } else {
            self.messages.push(DisplayMessage::new_text(
                Role::System,
                format!("IRC peers: {}. Format: <peer> <message>", peers.join(", ")),
            ));
        }
    }

    pub(crate) fn render_irc_compose(&self, f: &mut Frame) {
        if !self.irc_compose_open {
            return;
        }
        let area = f.area();
        let w = (area.width * 60 / 100).max(40).min(area.width);
        let h = 7u16.min(area.height);
        let x = area.width.saturating_sub(w) / 2;
        let y = area.height / 3;
        let modal_area = Rect::new(x, y, w, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .title(" IRC message (Esc cancel, Enter send) ");

        let inner = block.inner(modal_area);
        f.render_widget(ratatui::widgets::Clear, modal_area);
        f.render_widget(block, modal_area);

        let hint = Line::from(vec![
            Span::styled("Format: ", Style::default().fg(Color::DarkGray)),
            Span::raw("<peer> <message>  "),
            Span::styled("(use ", Style::default().fg(Color::DarkGray)),
            Span::styled("all", Style::default().fg(Color::Yellow)),
            Span::styled(" to broadcast)", Style::default().fg(Color::DarkGray)),
        ]);
        if inner.height > 0 {
            f.render_widget(
                Paragraph::new(hint),
                Rect::new(inner.x, inner.y, inner.width, 1),
            );
        }

        let input_line = Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Magenta)),
            Span::styled(&self.irc_compose_input, Style::default().fg(Color::White)),
            Span::styled(
                "▌",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]);
        if inner.height > 1 {
            f.render_widget(
                Paragraph::new(input_line),
                Rect::new(
                    inner.x,
                    inner.y + 1,
                    inner.width,
                    inner.height.saturating_sub(1),
                ),
            );
        }
    }
}

fn format_irc_line(msg: &IrcMessage) -> String {
    format!("[IRC] {} → {}: {}", msg.from, msg.to, msg.body)
}

#[cfg(test)]
mod tests {
    use super::format_irc_line;
    use crate::tui::app::App;
    use crate::tui::types::AgentCommand;
    use pawan::agent::{IrcHub, IrcMessage, Role};
    use pawan::config::TuiConfig;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    /// Minimal App harness (mirrors `render.rs` `test_app`, but keeps `cmd_rx` for dispatch checks).
    fn test_app<'a>() -> (App<'a>, mpsc::UnboundedReceiver<AgentCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_hub = IrcHub::new();
        let irc_relay = Arc::new(Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        app.show_welcome = false;
        (app, cmd_rx)
    }

    fn test_app_with_hub<'a>(irc_hub: &IrcHub) -> (App<'a>, mpsc::UnboundedReceiver<AgentCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (_event_tx, event_rx) = mpsc::unbounded_channel();
        let irc_relay = Arc::new(Mutex::new(irc_hub.join("main")));
        let mut app = App::new(
            TuiConfig::default(),
            "test-model".to_string(),
            cmd_tx,
            event_rx,
            irc_relay,
        );
        app.show_welcome = false;
        (app, cmd_rx)
    }

    fn last_system_text(app: &App<'_>) -> String {
        app.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::System)
            .expect("system message")
            .text_content()
    }

    #[test]
    fn format_irc_line_formats_message() {
        use chrono::Utc;

        let msg = IrcMessage {
            from: "worker".into(),
            to: "main".into(),
            body: "ping".into(),
            timestamp: Utc::now(),
        };
        assert_eq!(format_irc_line(&msg), "[IRC] worker → main: ping");
    }

    #[test]
    fn submit_irc_compose_empty_line_closes_without_dispatch() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "   ".into();
        let messages_before = app.messages.len();

        app.submit_irc_compose();

        assert!(!app.irc_compose_open);
        assert!(app.irc_compose_input.is_empty());
        assert_eq!(app.messages.len(), messages_before);
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn submit_irc_compose_bad_format_shows_usage() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "nopeer".into();

        app.submit_irc_compose();

        assert!(app.irc_compose_open);
        assert_eq!(app.irc_compose_input, "nopeer");
        assert!(last_system_text(&app).contains("IRC usage"));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn submit_irc_compose_missing_body_shows_usage() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "peer   ".into();

        app.submit_irc_compose();

        assert!(app.irc_compose_open);
        assert!(last_system_text(&app).contains("IRC usage"));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn submit_irc_compose_success_dispatches_and_closes() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "worker  hello world".into();

        app.submit_irc_compose();

        assert!(!app.irc_compose_open);
        assert!(app.irc_compose_input.is_empty());
        assert_eq!(app.status, "IRC → worker");
        assert!(last_system_text(&app).contains("IRC queued to worker: hello world"));

        match cmd_rx.try_recv().expect("IrcSend command") {
            AgentCommand::IrcSend { to, body } => {
                assert_eq!(to, "worker");
                assert_eq!(body, "hello world");
            }
            _ => panic!("expected IrcSend"),
        }
    }

    #[test]
    fn open_irc_compose_sets_state_and_lists_peers() {
        let hub = IrcHub::new();
        let _worker = hub.join("worker");
        let (mut app, _cmd_rx) = test_app_with_hub(&hub);

        app.open_irc_compose();

        assert!(app.irc_compose_open);
        assert!(app.irc_compose_input.is_empty());
        let hint = last_system_text(&app);
        assert!(hint.contains("IRC peers:"));
        assert!(hint.contains("worker"));
    }

    #[test]
    fn open_irc_compose_without_peers_shows_hint() {
        let (mut app, _cmd_rx) = test_app();

        app.open_irc_compose();

        assert!(app.irc_compose_open);
        assert!(last_system_text(&app).contains("no other agents registered"));
    }

    #[test]
    fn poll_irc_inbox_surfaces_formatted_line() {
        let hub = IrcHub::new();
        let worker = hub.join("worker");
        let (mut app, _cmd_rx) = test_app_with_hub(&hub);

        worker.send("main", "ping").expect("deliver");
        app.poll_irc_inbox();

        assert_eq!(app.status, "IRC from worker");
        assert_eq!(
            app.messages.last().expect("inbox message").text_content(),
            "[IRC] worker → main: ping"
        );
    }

    #[test]
    fn render_irc_compose_smoke() {
        let (mut app, _cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "all hello".into();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render_irc_compose(f)).unwrap();

        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("IRC message"));
        assert!(text.contains("all hello"));
    }

    #[test]
    fn open_irc_compose_opens_modal_with_cleared_input() {
        let (mut app, _cmd_rx) = test_app();
        assert!(!app.irc_compose_open);

        app.open_irc_compose();

        assert!(app.irc_compose_open);
        assert!(app.irc_compose_input.is_empty());
    }

    #[test]
    fn submit_irc_compose_whitespace_only_dismisses_without_irc_status() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.status = "Ready".into();
        app.irc_compose_input = "	  
"
        .into();

        app.submit_irc_compose();

        assert!(!app.irc_compose_open);
        assert_eq!(app.status, "Ready");
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn submit_irc_compose_to_all_dispatches_irc_send() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "all team sync".into();

        app.submit_irc_compose();

        match cmd_rx.try_recv().expect("IrcSend command") {
            AgentCommand::IrcSend { to, body } => {
                assert_eq!(to, "all");
                assert_eq!(body, "team sync");
            }
            _ => panic!("expected IrcSend"),
        }
    }

    #[test]
    fn format_irc_line_alice_to_bob() {
        use chrono::Utc;

        let msg = IrcMessage {
            from: "alice".into(),
            to: "bob".into(),
            body: "hi".into(),
            timestamp: Utc::now(),
        };
        assert_eq!(format_irc_line(&msg), "[IRC] alice → bob: hi");
    }

    #[test]
    fn submit_irc_compose_single_token_peer_shows_usage() {
        let (mut app, mut cmd_rx) = test_app();
        app.irc_compose_open = true;
        app.irc_compose_input = "lonely-peer".into();

        app.submit_irc_compose();

        assert!(app.irc_compose_open);
        assert!(last_system_text(&app).contains("IRC usage"));
        assert!(cmd_rx.try_recv().is_err());
    }

    #[test]
    fn poll_irc_inbox_drains_multiple_messages() {
        let hub = IrcHub::new();
        let worker = hub.join("worker");
        let (mut app, _cmd_rx) = test_app_with_hub(&hub);

        worker.send("main", "one").expect("deliver");
        worker.send("main", "two").expect("deliver");
        app.poll_irc_inbox();

        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.status, "IRC from worker");
        app.poll_irc_inbox();
        assert_eq!(app.messages.len(), 2);
    }
}
