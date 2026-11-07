//! Every agent hc knows about and what it last said it was doing, followed live from `hc events`. Status comes
//! from the agents themselves (OSC 7501, which hc answers and stores), so plain shells have none and aren't here

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::process::{Child, ChildStdout, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::{hc, waiting};

/// Wait before starting `hc events` again after it ends, e.g. while the core restarts for an update
const RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
pub struct Agent {
    pub addr: String,
    pub name: Option<String>,
    /// idle, working, done, blocked or error, or waiting: done but with background tasks still running (see
    /// waiting.rs)
    pub state: String,
    /// Why it's blocked: permission, question or auth
    pub kind: Option<String>,
    /// One line from the agent about it
    pub msg: Option<String>,
    /// When it last reported, ms since the epoch
    pub since: u64,
    /// You've looked at its window since it last changed state
    pub seen: bool,
}

impl Agent {
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.addr)
    }

    /// The machine it's on: the name hc knows it by, or this one's hostname
    pub fn device(&self) -> String {
        self.addr.rsplit_once('/').map_or_else(|| hc::HOST.clone(), |(machine, _)| machine.to_string())
    }

    /// How much it wants you, most first, None if it doesn't or you've already looked
    pub fn urgency(&self) -> Option<u8> {
        hc::urgency(&self.state).filter(|_| !self.seen)
    }
}

pub struct Agents {
    stream: Option<(Child, ChildStdout)>,
    /// When to start the stream again after it ended
    retry_at: Option<Instant>,
    line: Vec<u8>,
    /// Every live terminal by address
    terms: BTreeMap<String, Term>,
    /// Class of the focused window, "hc:<address>" for an agent's
    focused: String,
}

struct Term {
    name: Option<String>,
    /// None until it reports a status, and for programs that never do
    agent: Option<Agent>,
    /// Its turn ended with background tasks still running, from the label `lookout waiting` keeps
    waiting: bool,
}

impl Agents {
    pub fn new() -> Agents {
        let mut agents =
            Agents { stream: None, retry_at: None, line: Vec::new(), terms: BTreeMap::new(), focused: String::new() };
        agents.start();
        agents
    }

    /// Readable when hc has said something
    pub fn fd(&self) -> Option<RawFd> {
        self.stream.as_ref().map(|(_, out)| out.as_raw_fd())
    }

    /// When the stream should be started again, if it's down
    pub fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    fn start(&mut self) {
        self.retry_at = None;
        let child = hc::command(&["events", "--json"]).stdin(Stdio::null()).stdout(Stdio::piped()).spawn();
        match child {
            Ok(mut child) => {
                let out = child.stdout.take().unwrap();
                self.stream = Some((child, out));
            }
            Err(err) => {
                eprintln!("lookout: can't run hc events: {err}");
                self.stop();
            }
        }
    }

    /// Ends the stream and tries again shortly. What's known is kept until the new snapshot replaces it
    fn stop(&mut self) {
        if let Some((mut child, _)) = self.stream.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.line.clear();
        self.retry_at = Some(Instant::now() + RETRY);
    }

    pub fn retry(&mut self, now: Instant) {
        if self.retry_at.is_some_and(|at| now >= at) {
            self.start();
        }
    }

    /// Reads what's arrived, true if the agents changed
    pub fn read(&mut self) -> bool {
        let Some((_, out)) = &mut self.stream else { return false };
        let mut buf = [0u8; 16384];
        let n = match out.read(&mut buf) {
            Ok(0) => {
                self.stop();
                return false;
            }
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => return false,
            Err(_) => {
                self.stop();
                return false;
            }
        };
        let before = self.list();
        self.line.extend_from_slice(&buf[..n]);
        while let Some(end) = self.line.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.line.drain(..=end).collect();
            if let Ok(msg) = serde_json::from_slice::<Value>(&line) {
                self.apply(&msg);
            }
        }
        self.list() != before
    }

    fn apply(&mut self, msg: &Value) {
        // The first line is a snapshot of everything
        if let Some(records) = msg["terminals"].as_array() {
            let old = std::mem::take(&mut self.terms);
            for record in records.iter().filter(|t| t["state"] == "live") {
                let Some(addr) = hc::str(&record["address"]) else { continue };
                let mut term = Term {
                    name: hc::str(&record["name"]),
                    agent: agent(&addr, record, &record["status"]),
                    waiting: waiting(&record["labels"]),
                };
                // Keep what's been seen across a reconnect if nothing changed meanwhile
                if let (Some(new), Some(Term { agent: Some(old), .. })) = (&mut term.agent, old.get(&addr)) {
                    new.seen = old.seen && old.state == new.state;
                }
                self.terms.insert(addr, term);
            }
            self.mark_focused_seen();
            return;
        }
        match msg["method"].as_str() {
            Some("event") => {}
            // Fell behind and events were dropped, so start over from a fresh snapshot
            Some("missed") => return self.stop(),
            _ => return,
        }
        let params = &msg["params"];
        let (Some(addr), Some(kind)) = (hc::str(&params["terminal"]), params["type"].as_str()) else { return };
        let data = &params["data"];
        match kind {
            // The whole record
            "created" | "restarted" => {
                let term = Term {
                    name: hc::str(&data["name"]),
                    agent: agent(&addr, data, &data["status"]),
                    waiting: waiting(&data["labels"]),
                };
                self.terms.insert(addr, term);
            }
            "labels" => {
                if let Some(term) = self.terms.get_mut(&addr) {
                    term.waiting = waiting(&data["labels"]);
                }
            }
            "finished" | "removed" => {
                self.terms.remove(&addr);
            }
            "status" => {
                let Some(term) = self.terms.get_mut(&addr) else { return };
                let mut new = agent(&addr, &Value::Null, &data["status"]);
                // The same state again (e.g. a new message while working) doesn't make it want you again
                if let (Some(new), Some(old)) = (&mut new, &term.agent) {
                    if old.state == new.state {
                        new.seen = old.seen;
                    }
                }
                term.agent = new;
                self.mark_focused_seen();
            }
            _ => {}
        }
    }

    /// A window was focused: looking at an agent's window means you've seen what it said
    pub fn focus(&mut self, class: &str) -> bool {
        let before = self.list();
        self.focused = class.to_string();
        self.mark_focused_seen();
        self.list() != before
    }

    fn mark_focused_seen(&mut self) {
        if let Some(addr) = self.focused.strip_prefix("hc:") {
            if let Some(Term { agent: Some(agent), .. }) = self.terms.get_mut(addr) {
                agent.seen = true;
            }
        }
    }

    /// The agents, the ones that want you most first, then working ones, each oldest first
    pub fn list(&self) -> Vec<Agent> {
        let mut agents: Vec<Agent> = self
            .terms
            .values()
            .filter_map(|term| {
                let mut agent = term.agent.clone()?;
                agent.name = term.name.clone();
                if term.waiting && agent.state == "done" {
                    agent.state = "waiting".into();
                }
                Some(agent)
            })
            .collect();
        agents.sort_by_key(|a| (a.urgency().unwrap_or(u8::MAX), !matches!(a.state.as_str(), "working" | "waiting"), a.since));
        agents
    }
}

/// Whether a terminal's labels say its agent is waiting on background tasks
fn waiting(labels: &Value) -> bool {
    hc::str(&labels[waiting::LABEL]).is_some()
}

/// An agent from a terminal `record` and its `status`, None if it has never reported one
fn agent(addr: &str, record: &Value, status: &Value) -> Option<Agent> {
    let state = hc::str(&status["state"])?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    Some(Agent {
        addr: addr.to_string(),
        name: None,
        state,
        kind: hc::str(&status["kind"]),
        msg: hc::str(&status["msg"]),
        since: record["status_ms"].as_u64().unwrap_or(now),
        seen: false,
    })
}
