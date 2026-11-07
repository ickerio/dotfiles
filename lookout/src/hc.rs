//! herdr-core (`hc`) holds the terminals agents run in, on this machine and the ones added with `hc machine add`.
//! Everything here goes through its CLI, whose JSON is the stable part

use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use serde_json::Value;

/// This machine's name, which is what agents on it are labelled with
pub static HOST: LazyLock<String> =
    LazyLock::new(|| fs::read_to_string("/proc/sys/kernel/hostname").map_or("here".into(), |h| h.trim().to_string()));

/// `hc` with `args`, minus any Claude Code session variables. If this starts the core, the core and every agent
/// it runs inherit this environment, and an agent that thinks it's inside another Claude session doesn't save
/// its transcript
pub fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new("hc");
    cmd.args(args);
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("CLAUDE") {
            cmd.env_remove(key);
        }
    }
    cmd
}

/// Runs `hc` and parses what it printed. If it failed, what hc said went wrong, to show since a bind has nowhere to
/// print it
pub fn json(args: &[&str]) -> Result<Value, String> {
    let out = call(args)?;
    serde_json::from_slice(&out).map_err(|err| format!("hc {} said something odd: {err}", args[0]))
}

/// Runs `hc` and returns what it printed, or what it said went wrong
pub fn call(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = command(args).arg("--json").output().map_err(|err| format!("can't run hc: {err}"))?;
    if !out.status.success() {
        let err: Value = serde_json::from_slice(&out.stderr).unwrap_or_default();
        return Err(str(&err["error"]["message"]).unwrap_or_else(|| format!("hc {} failed", args[0])));
    }
    Ok(out.stdout)
}

/// A live terminal as `hc ls` sees it
pub struct Term {
    /// "t3" here, "pc/t3" on a machine named pc
    pub addr: String,
    pub name: Option<String>,
    /// What's running in it: the agent's own name for itself if it reports status ("claude-code"), otherwise the
    /// program in the foreground ("bash")
    pub program: Option<String>,
    /// idle, working, done, blocked or error, if it reports status
    pub status: Option<String>,
    pub cwd: String,
}

impl Term {
    /// The name if it has one, else its address
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.addr)
    }

    /// The machine it's on, None for this one
    pub fn machine(&self) -> Option<&str> {
        self.addr.rsplit_once('/').map(|(machine, _)| machine)
    }

    /// A shell waiting at its prompt: what's left when an agent exits, or what `--keep` brings back after a reboot
    pub fn at_prompt(&self) -> bool {
        self.status.is_none() && self.program.as_deref().is_some_and(|p| SHELLS.contains(&p))
    }
}

const SHELLS: [&str; 5] = ["bash", "zsh", "fish", "sh", "nu"];

pub fn list() -> Vec<Term> {
    let Ok(ls) = json(&["ls"]) else { return Vec::new() };
    let terms = ls["terminals"].as_array().map(Vec::as_slice).unwrap_or_default();
    terms
        .iter()
        .filter(|t| t["state"] == "live")
        .map(|t| Term {
            addr: str(&t["address"]).unwrap_or_default(),
            name: str(&t["name"]),
            program: str(&t["agent"]),
            status: str(&t["status"]),
            cwd: str(&t["cwd"]).unwrap_or_default(),
        })
        .collect()
}

/// Machines added with `hc machine add` that are connected, by name. One that's asleep or out of reach makes every
/// call to it wait out a timeout first, so it's left out until hc reconnects on its own
pub fn machines() -> Vec<String> {
    let Ok(ls) = json(&["machine", "ls"]) else { return Vec::new() };
    let machines = ls["machines"].as_array().map(Vec::as_slice).unwrap_or_default();
    machines.iter().filter(|m| m["state"] == "connected").filter_map(|m| str(&m["name"])).collect()
}

/// `sh -c script` with `args`, here or on `machine`
pub fn sh(machine: Option<&str>, script: &str, args: &[&str]) -> Command {
    let mut cmd = match machine {
        Some(m) => command(&["exec", "--on", m, "sh", "-c", script, "sh"]),
        None => {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", script, "sh"]);
            cmd
        }
    };
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    cmd
}

pub fn str(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(String::from)
}

/// How much an agent in `status` wants you, most first, None if it doesn't
pub fn urgency(status: &str) -> Option<u8> {
    match status {
        "blocked" => Some(0),
        "error" => Some(1),
        "done" => Some(2),
        _ => None,
    }
}

/// "~/x" for a path under $HOME
pub fn tilde(path: &str) -> String {
    match env::var("HOME") {
        Ok(home) if path == home => "~".into(),
        Ok(home) if path.starts_with(&format!("{home}/")) => format!("~{}", &path[home.len()..]),
        _ => path.into(),
    }
}
