//! IRC-style message relay between named agents in one process.
//!
//! Adapted from oh-my-pi's agent-to-agent messaging pattern: peers are
//! discovered by id, messages are plain prose routed over in-memory channels
//! (no IRC wire protocol). Reply generation and history injection are left
//! for a later integration pass.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// A single routed message between agents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrcMessage {
    pub from: String,
    pub to: String,
    pub body: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Default)]
struct IrcRouter {
    inboxes: HashMap<String, mpsc::UnboundedSender<IrcMessage>>,
}

/// Shared routing hub — clone and call [`IrcHub::join`] for each live agent.
#[derive(Clone, Default)]
pub struct IrcHub(Arc<Mutex<IrcRouter>>);

impl IrcHub {
    /// Create an empty hub (orchestrator + subagents share one instance per process).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `agent_id` and return a handle for send/receive.
    pub fn join(&self, agent_id: impl Into<String>) -> IrcRelay {
        IrcRelay::join_with_hub(agent_id, self.clone())
    }
}

/// Per-agent IRC endpoint: send to peers and poll the local inbox.
pub struct IrcRelay {
    agent_id: String,
    hub: IrcHub,
    inbox_rx: mpsc::UnboundedReceiver<IrcMessage>,
}

impl IrcRelay {
    fn join_with_hub(agent_id: impl Into<String>, hub: IrcHub) -> Self {
        let agent_id = agent_id.into();
        let (tx, rx) = mpsc::unbounded_channel();
        hub.0
            .lock()
            .expect("irc router lock")
            .inboxes
            .insert(agent_id.clone(), tx);
        Self {
            agent_id,
            hub,
            inbox_rx: rx,
        }
    }

    /// This agent's address (e.g. `"main"`, `"subagent-explore"`).
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Send prose to `to`. Use `"all"` to broadcast to every other registered peer.
    ///
    /// Stub: delivery is synchronous over mpsc; auto-replies are not generated yet.
    pub fn send(&self, to: &str, body: impl Into<String>) -> Result<IrcMessage, String> {
        let body = body.into();
        if body.trim().is_empty() {
            return Err("message body is empty".to_string());
        }
        let to = to.trim();
        if to.is_empty() {
            return Err("recipient is required".to_string());
        }

        let msg = IrcMessage {
            from: self.agent_id.clone(),
            to: to.to_string(),
            body,
            timestamp: Utc::now(),
        };
        self.deliver(&msg)?;
        Ok(msg)
    }

    fn deliver(&self, msg: &IrcMessage) -> Result<(), String> {
        let router = self.hub.0.lock().expect("irc router lock");

        if msg.to == "all" {
            let mut delivered = 0usize;
            for (id, tx) in &router.inboxes {
                if id == &msg.from {
                    continue;
                }
                if tx.send(msg.clone()).is_ok() {
                    delivered += 1;
                }
            }
            if delivered == 0 {
                return Err("no peers available for broadcast".to_string());
            }
            return Ok(());
        }

        if msg.to == msg.from {
            return Err("cannot message self".to_string());
        }

        let Some(tx) = router.inboxes.get(&msg.to) else {
            return Err(format!("unknown peer: {}", msg.to));
        };

        tx.send(msg.clone())
            .map_err(|_| format!("peer '{}' is unavailable", msg.to))
    }

    /// Non-blocking poll of this agent's inbox.
    pub fn try_receive(&mut self) -> Option<IrcMessage> {
        self.inbox_rx.try_recv().ok()
    }

    /// Blocking receive stub — delegates to [`Self::try_receive`] until integrated with async loops.
    pub async fn receive(&mut self) -> Option<IrcMessage> {
        self.try_receive()
    }

    /// Other registered agent ids (excluding self), sorted for stable display.
    pub fn list_peers(&self) -> Vec<String> {
        let router = self.hub.0.lock().expect("irc router lock");
        let mut peers: Vec<String> = router
            .inboxes
            .keys()
            .filter(|id| *id != &self.agent_id)
            .cloned()
            .collect();
        peers.sort();
        peers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_direct_message() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let mut worker = hub.join("worker");

        main.send("worker", "ping").expect("send");
        let msg = worker.try_receive().expect("inbox");
        assert_eq!(msg.from, "main");
        assert_eq!(msg.to, "worker");
        assert_eq!(msg.body, "ping");
    }

    #[test]
    fn broadcast_skips_sender() {
        let hub = IrcHub::new();
        let mut main = hub.join("main");
        let mut a = hub.join("a");
        let mut b = hub.join("b");

        main.send("all", "hello team").expect("broadcast");
        assert!(a.try_receive().is_some());
        assert!(b.try_receive().is_some());
        assert!(main.try_receive().is_none());
    }

    #[test]
    fn list_peers_returns_registered_ids() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        hub.join("zebra");
        hub.join("alpha");

        assert_eq!(main.list_peers(), vec!["alpha", "zebra"]);
    }

    #[test]
    fn send_to_unknown_peer_errors() {
        let hub = IrcHub::new();
        let main = hub.join("main");

        let err = main.send("ghost", "hello").unwrap_err();
        assert!(err.contains("unknown peer: ghost"));
    }

    #[test]
    fn duplicate_join_replaces_inbox() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let mut first = hub.join("worker");
        let mut second = hub.join("worker");

        main.send("worker", "after rejoin").expect("send");
        assert!(first.try_receive().is_none());
        let msg = second.try_receive().expect("new inbox");
        assert_eq!(msg.body, "after rejoin");
    }

    #[test]
    fn message_timestamp_set() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let _worker = hub.join("worker");
        let before = Utc::now();
        let msg = main.send("worker", "timed").expect("send");
        let after = Utc::now();

        assert!(msg.timestamp >= before);
        assert!(msg.timestamp <= after);
    }

    #[test]
    fn try_receive_on_empty_inbox_returns_none() {
        let hub = IrcHub::new();
        let mut main = hub.join("main");
        assert!(main.try_receive().is_none());
    }

    #[test]
    fn send_empty_body_returns_err() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let _peer = hub.join("worker");
        let err = main.send("worker", "   ").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn send_to_self_returns_err() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let err = main.send("main", "hello").unwrap_err();
        assert!(err.contains("cannot message self"));
    }

    #[test]
    fn broadcast_with_no_other_peers_errors() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let err = main.send("all", "solo").unwrap_err();
        assert!(err.contains("no peers available"));
    }

    #[test]
    fn list_peers_empty_when_only_self_registered() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        assert!(main.list_peers().is_empty());
    }

    #[test]
    fn try_receive_drains_direct_messages_in_order() {
        let hub = IrcHub::new();
        let main = hub.join("main");
        let mut worker = hub.join("worker");
        main.send("worker", "first").unwrap();
        main.send("worker", "second").unwrap();
        assert_eq!(worker.try_receive().unwrap().body, "first");
        assert_eq!(worker.try_receive().unwrap().body, "second");
        assert!(worker.try_receive().is_none());
    }
}
