//! `lookout waiting`, a Claude Code Stop hook: when a turn ends with background tasks still running (a build, or a
//! wait on a long job), the agent isn't done, it's waiting on them. Claude still reports done, so this labels its hc
//! terminal `waiting=N`, and the board shows it as waiting rather than wanting you. The next turn's end, after a
//! task finishes and wakes it, labels it again.
//!
//! Claude writes each background task's output to `<tmp>/claude-<uid>/<project>/<session>/tasks/<id>.output`, and
//! the task holds that file open until it ends, so the ones still open are the ones still running.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;

use crate::hc;

/// The label lookout reads, see agents.rs
pub const LABEL: &str = "waiting";

pub fn run() {
    // Only an agent in an hc terminal has somewhere to say it
    let Ok(terminal) = env::var("HC_TERMINAL_ID") else { return };
    let mut input = String::new();
    let _ = io::stdin().read_to_string(&mut input);
    let session = serde_json::from_str::<Value>(&input).ok().and_then(|v| hc::str(&v["session_id"]));
    let count = session.map_or(0, |session| running(&session));
    let label = if count > 0 { format!("{LABEL}={count}") } else { format!("{LABEL}=") };
    // Quietly: a hook's output lands in the session
    let _ = hc::command(&["label", &terminal, &label]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// How many of the session's background tasks are still running
fn running(session: &str) -> usize {
    let Ok(uid) = fs::metadata("/proc/self").map(|m| m.uid()) else { return 0 };
    let tmp = env::var_os("CLAUDE_CODE_TMPDIR").or_else(|| env::var_os("TMPDIR")).map_or("/tmp".into(), PathBuf::from);
    let Ok(projects) = fs::read_dir(tmp.join(format!("claude-{uid}"))) else { return 0 };
    let outputs: Vec<PathBuf> = projects
        .flatten()
        .filter_map(|project| fs::read_dir(project.path().join(session).join("tasks")).ok())
        .flatten()
        .flatten()
        .map(|task| task.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "output"))
        .collect();
    if outputs.is_empty() {
        return 0;
    }
    let open = open_files();
    outputs.iter().filter(|path| open.contains(*path)).count()
}

/// Every file some process has open, as far as this user can see
fn open_files() -> HashSet<PathBuf> {
    let Ok(procs) = fs::read_dir("/proc") else { return HashSet::new() };
    procs
        .flatten()
        .filter(|p| p.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
        .filter_map(|p| fs::read_dir(p.path().join("fd")).ok())
        .flatten()
        .flatten()
        .filter_map(|fd| fs::read_link(fd.path()).ok())
        .collect()
}
