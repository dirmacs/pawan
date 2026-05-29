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
        let peers = self
            .irc_relay
            .lock()
            .expect("irc relay lock")
            .list_peers();
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
                Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1)),
            );
        }
    }
}

fn format_irc_line(msg: &IrcMessage) -> String {
    format!("[IRC] {} → {}: {}", msg.from, msg.to, msg.body)
}
