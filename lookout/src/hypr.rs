//! Hyprland's IPC: its event socket says which window is focused, and hyprctl finds and focuses windows

use std::env;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Wait before connecting again, e.g. while Hyprland reloads
const RETRY: Duration = Duration::from_secs(5);

pub enum Event {
    /// A window was focused, with its class
    Focus(String),
    /// Something moved on screen: a workspace or window change, or a launcher. Peeking ends, and what covers the
    /// board may have changed
    Moved,
}

pub struct Hypr {
    events: Option<UnixStream>,
    retry_at: Option<Instant>,
    line: Vec<u8>,
    /// The focused window's class from "activewindow", then its address from the "activewindowv2" after it
    class: String,
    address: String,
}

impl Hypr {
    pub fn new() -> Hypr {
        // Starting from what's focused now, so the first event about the same window isn't taken for a new one
        let window = request("j/activewindow");
        let field = |key: &str| window.as_ref().and_then(|w| w[key].as_str()).unwrap_or_default().to_string();
        let mut hypr = Hypr { events: None, retry_at: None, line: Vec::new(), class: field("class"), address: field("address").trim_start_matches("0x").to_string() };
        hypr.connect();
        hypr
    }

    pub fn fd(&self) -> Option<RawFd> {
        self.events.as_ref().map(|s| s.as_raw_fd())
    }

    pub fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    pub fn retry(&mut self, now: Instant) {
        if self.retry_at.is_some_and(|at| now >= at) {
            self.connect();
        }
    }

    fn connect(&mut self) {
        self.retry_at = None;
        match socket(".socket2.sock").and_then(UnixStream::connect) {
            Ok(stream) => self.events = Some(stream),
            Err(_) => self.retry_at = Some(Instant::now() + RETRY),
        }
    }

    pub fn read(&mut self) -> Vec<Event> {
        let Some(stream) = &mut self.events else { return Vec::new() };
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(n) if n > 0 => self.line.extend_from_slice(&buf[..n]),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => return Vec::new(),
            _ => {
                self.events = None;
                self.line.clear();
                self.retry_at = Some(Instant::now() + RETRY);
                return Vec::new();
            }
        }
        let mut events = Vec::new();
        while let Some(end) = self.line.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.line.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]);
            let Some((name, data)) = line.split_once(">>") else { continue };
            match name {
                // class,title. Classes never have commas, titles might. It's sent again whenever the title changes,
                // which an agent's spinner does several times a second, so only a new address is a new window
                "activewindow" => self.class = data.split(',').next().unwrap_or_default().to_string(),
                "activewindowv2" if data != self.address => {
                    self.address = data.to_string();
                    events.push(Event::Focus(self.class.clone()));
                }
                "workspace" | "openwindow" | "closewindow" | "movewindow" | "fullscreen" | "changefloatingmode"
                | "focusedmon" | "activespecial" => events.push(Event::Moved),
                "openlayer" if data != "lookout" => events.push(Event::Moved),
                _ => {}
            }
        }
        events
    }
}

/// $XDG_RUNTIME_DIR/hypr/<instance>/<name>
fn socket(name: &str) -> io::Result<PathBuf> {
    let runtime = env::var("XDG_RUNTIME_DIR").map_err(|_| io::Error::other("XDG_RUNTIME_DIR not set"))?;
    let instance = env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(|_| io::Error::other("not running under Hyprland"))?;
    Ok(PathBuf::from(runtime).join("hypr").join(instance).join(name))
}

/// One request on the command socket, e.g. "j/clients", answered with JSON
fn request(what: &str) -> Option<Value> {
    let mut stream = UnixStream::connect(socket(".socket.sock").ok()?).ok()?;
    stream.write_all(what.as_bytes()).ok()?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).ok()?;
    serde_json::from_slice(&reply).ok()
}

/// Mapped windows as (address, class)
pub fn windows() -> Vec<(String, String)> {
    let Some(Value::Array(clients)) = request("j/clients") else { return Vec::new() };
    clients
        .iter()
        .filter(|c| c["mapped"] == true)
        .filter_map(|c| Some((c["address"].as_str()?.to_string(), c["class"].as_str()?.to_string())))
        .collect()
}

/// Whether windows cover the board: a fullscreen window, or any tiled one, since tiling fills the screen. Floating
/// windows might not, so they don't count
pub fn covered() -> bool {
    let Some(workspace) = request("j/activeworkspace") else { return false };
    if workspace["hasfullscreen"] == true {
        return true;
    }
    let Some(Value::Array(clients)) = request("j/clients") else { return false };
    clients.iter().any(|c| {
        c["workspace"]["id"] == workspace["id"] && c["mapped"] == true && c["hidden"] != true && c["floating"] == false
    })
}

/// The focused window's class right now. Events only say when it changes
pub fn focused_class() -> Option<String> {
    request("j/activewindow")?["class"].as_str().map(String::from)
}

/// Focuses the window at `address`, switching to its workspace
pub fn focus(address: &str) {
    // With a Lua config, dispatch takes Lua
    let lua = format!("hl.dsp.focus({{ window = \"address:{address}\" }})");
    let _ = Command::new("hyprctl").args(["dispatch", &lua]).output();
}
