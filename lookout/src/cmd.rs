//! What the Hyprland binds run: ask the bar's daemon to pick an agent, start one or end one, jump to the one that
//! wants you, and the loop each agent's foot window runs. Then the slow work the daemon's picker does off its loop.
//! An agent's window has the class "hc:<address>", which is how it's found again

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{exit, Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::desktop;
use crate::hc::{self, Term};
use crate::hypr;
use crate::sys;

use serde_json::Value;

/// Lines of history a window shows when it opens
const HISTORY: &str = "10000";
/// $HOME, then git repos this deep under it, skipping hidden folders and node_modules
const PROJECTS: &str = r#"echo "$HOME"; find "$HOME" -maxdepth 4 \( -path "$HOME/.*" -o -name node_modules \) -prune -o -name .git -print 2>/dev/null"#;
/// For each name and folder given, "name id" of the newest Claude session in that folder with that name, if there is
/// one. Claude keeps a folder's sessions in ~/.claude/projects/ under the path with everything but letters and digits
/// made a dash, and `claude -n` writes the name into the session as "customTitle". Only the newest few are searched,
/// since an agent that's been running is among them
const SESSIONS: &str = r#"while [ $# -ge 2 ]; do
  dir="$HOME/.claude/projects/$(printf %s "$2" | tr -c 'A-Za-z0-9' -)"
  for f in $(ls -t "$dir" 2>/dev/null | grep '\.jsonl$' | head -20); do
    if grep -q -m1 -F "\"customTitle\":\"$1\"" "$dir/$f"; then echo "$1 ${f%.jsonl}"; break; fi
  done
  shift 2
done"#;

/// What a terminal that's only a shell at its prompt has in front
const SHELLS: [&str; 5] = ["bash", "zsh", "fish", "sh", "nu"];

/// What `lookout pick`, `new` and `kill` ask the daemon to open its picker for, sent with sys::SIGCMD
pub const PICK: u64 = 1;
pub const NEW: u64 = 2;
pub const END: u64 = 3;

/// Asks the daemon to open its picker, which is already running, so it opens at once
pub fn ask(what: u64) {
    match daemon() {
        Some(pid) => {
            let _ = sys::send_signal(pid, sys::SIGCMD, what);
        }
        None => {
            eprintln!("lookout: the bar isn't running, start it with `lookout`");
            exit(1);
        }
    }
}

/// What `hc ls` leaves out about a terminal
pub struct Detail {
    /// What's running in it now. `hc ls` names the program it started with, so a Claude typed into a shell looks
    /// like the shell there
    pub foreground: Option<String>,
}

/// For terminals that report no status: what's in front of each, and the Claude session each shell at its prompt
/// could resume, by address. Both take a round trip to the other machines, so they go at once, and it's run off the
/// daemon's loop
pub fn shells(terms: &[Term]) -> (HashMap<String, Detail>, HashMap<String, String>) {
    let asked: Vec<(String, Child)> = terms
        .iter()
        .filter_map(|t| {
            let mut get = hc::command(&["get", &t.addr, "--json"]);
            get.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            get.spawn().ok().map(|child| (t.addr.clone(), child))
        })
        .collect();
    let running = find_sessions(terms);
    let mut details = HashMap::new();
    for (addr, child) in asked {
        let Some(term) = child.wait_with_output().ok().and_then(|out| serde_json::from_slice::<Value>(&out.stdout).ok()) else { continue };
        details.insert(addr, Detail { foreground: hc::str(&term["foreground"]["name"]) });
    }
    let resumable = sessions(running, &details);
    (details, resumable)
}

/// Starts looking for the Claude session each named terminal could resume, every machine at once. Which of them are
/// really at a shell prompt is only known once their details are in, see sessions
fn find_sessions(terms: &[Term]) -> Vec<(Vec<&Term>, Child)> {
    let mut by_machine: HashMap<Option<&str>, Vec<&Term>> = HashMap::new();
    for term in terms.iter().filter(|t| t.name.is_some()) {
        by_machine.entry(term.machine()).or_default().push(term);
    }
    by_machine
        .into_iter()
        .filter_map(|(machine, terms)| {
            let args: Vec<&str> = terms.iter().flat_map(|t| [t.label(), t.cwd.as_str()]).collect();
            let child = hc::sh(machine, SESSIONS, &args).spawn().ok()?;
            Some((terms, child))
        })
        .collect()
}

/// The Claude sessions that shells at their prompt can resume, by address: an agent that exited, or a kept terminal
/// that came back as a shell after a reboot. Found by the terminal's name, which `lookout new` gave its Claude
fn sessions(running: Vec<(Vec<&Term>, Child)>, details: &HashMap<String, Detail>) -> HashMap<String, String> {
    // Only a shell that's in front, never one with Claude or anything else running in it, since resuming types
    // into it
    let at_prompt = |t: &Term| details.get(&t.addr).and_then(|d| d.foreground.as_deref()).is_some_and(|f| SHELLS.contains(&f));
    let mut found = HashMap::new();
    for (terms, child) in running {
        let Ok(out) = child.wait_with_output() else { continue };
        for (name, id) in String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_once(' ')) {
            if let Some(term) = terms.iter().find(|t| t.label() == name && at_prompt(t)) {
                found.insert(term.addr.clone(), id.to_string());
            }
        }
    }
    found
}

/// Goes to a terminal's window, opening one if it has none, and typing `claude --resume` into its shell first when
/// there's a session to pick up
pub fn go(addr: &str, label: &str, resume: Option<&str>) {
    // Asked again now rather than trusting the picker's look, which may be from before something else started in it
    let at_prompt = || {
        let term = hc::json(&["get", addr]).ok()?;
        hc::str(&term["foreground"]["name"]).filter(|f| SHELLS.contains(&f.as_str()))
    };
    if let Some(id) = resume.filter(|_| at_prompt().is_some()) {
        let _ = hc::command(&["send", addr, &format!("claude --resume {id}")]).stdout(Stdio::null()).status();
    }
    focus_or_open(addr, label);
}

/// Every project on every machine: $HOME and the git repos under it, the other machines first since they're where
/// agents mostly live
pub fn projects() -> Vec<(Option<String>, String)> {
    let machines: Vec<Option<String>> = hc::machines().into_iter().map(Some).chain([None]).collect();
    let running: Vec<_> = machines.iter().map(|m| hc::sh(m.as_deref(), PROJECTS, &[]).spawn()).collect();
    let mut places = Vec::new();
    for (machine, child) in machines.iter().zip(running) {
        let Ok(found) = child.and_then(|c| c.wait_with_output()) else { continue };
        let found = String::from_utf8_lossy(&found.stdout);
        let mut lines = found.lines();
        // Home too, for an agent that isn't about one project
        let Some(home) = lines.next() else { continue };
        let mut dirs: Vec<String> = lines.filter_map(|git| git.strip_suffix("/.git").map(String::from)).collect();
        dirs.sort();
        dirs.insert(0, home.to_string());
        places.extend(dirs.into_iter().map(|dir| (machine.clone(), dir)));
    }
    places
}

/// Starts a shell with Claude in it in `dir` on `machine`, named after the folder and unlike any of `taken`, and
/// opens its window
pub fn start(machine: Option<&str>, dir: &str, taken: &[String]) -> Result<(), String> {
    let base = term_name(&Path::new(dir).file_name().map_or(String::new(), |n| n.to_string_lossy().to_string()));
    let name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|n| !taken.contains(n)).unwrap();
    // A shell rather than Claude directly: it loads the login's PATH, and it's still there if Claude exits. Kept, so
    // that after a reboot it comes back as a shell the picker can resume Claude in
    let mut args = vec!["run", "--bg", "--keep", "--no-parent", "--cwd", dir, "--name", &name];
    if let Some(m) = machine {
        args.extend(["--on", m]);
    }
    let run = hc::json(&args).map_err(|err| format!("can't start {name}: {err}"))?;
    let addr = hc::str(&run["address"]).ok_or_else(|| format!("can't start {name}: hc didn't say where"))?;
    // Named, so the window's title and /resume show it, and the session can be found again by it
    let _ = hc::command(&["send", &addr, &format!("claude -n {name}")]).stdout(Stdio::null()).status();
    open(&addr, &name);
    Ok(())
}

/// Ends an agent for good, even a kept one
pub fn end(addr: &str, label: &str) -> Result<(), String> {
    hc::call(&["kill", addr]).map_err(|err| format!("can't end {label}: {err}"))?;
    let _ = hc::command(&["rm", addr]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    Ok(())
}

/// A name hc takes for a terminal, from a folder's: 1-32 of a-z 0-9 - _ . starting with a letter or digit, and not
/// shaped like an address such as t7. Room is left for "-2"
fn term_name(folder: &str) -> String {
    let name: String =
        folder.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect();
    let name: String = name.trim_start_matches(|c: char| !c.is_ascii_alphanumeric()).chars().take(28).collect();
    let address_like = name.strip_prefix('t').is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()));
    match name.as_str() {
        "" => "agent".into(),
        _ if address_like => format!("a-{name}"),
        _ => name,
    }
}

/// Focuses the agent that wants you most, skipping the one you're on, so pressing again moves to the next
pub fn next() {
    let focused = hypr::focused_class().unwrap_or_default();
    let here = focused.strip_prefix("hc:");
    // The daemon knows which you've already looked at. Without it, go by what hc says
    let wanting: Vec<(String, String)> = match seen_state() {
        Some(agents) => agents,
        None => {
            let mut terms = hc::list();
            terms.retain(|t| t.status.as_deref().and_then(hc::urgency).is_some());
            terms.sort_by_key(|t| t.status.as_deref().and_then(hc::urgency));
            terms.into_iter().map(|t| (t.addr.clone(), t.label().to_string())).collect()
        }
    };
    if let Some((addr, label)) = wanting.into_iter().find(|(addr, _)| Some(addr.as_str()) != here) {
        focus_or_open(&addr, &label);
    }
}

/// What an agent's window runs: attach, and attach again if hc lost it, until you detach or it ends
pub fn view(addr: &str) -> ! {
    loop {
        let status = hc::command(&["attach", "--history", HISTORY, addr]).status();
        match status.map(|s| s.code()) {
            // Detached with ctrl+\, or its program ended
            Ok(Some(0)) => exit(0),
            // No such terminal
            Ok(Some(3)) => {
                sleep(Duration::from_secs(3));
                exit(3);
            }
            _ => {
                eprintln!("lookout: lost {addr}, trying again");
                sleep(Duration::from_secs(3));
            }
        }
    }
}

pub fn focus_or_open(addr: &str, label: &str) {
    let class = format!("hc:{addr}");
    match hypr::windows().into_iter().find(|(_, c)| *c == class) {
        Some((address, _)) => hypr::focus(&address),
        None => open(addr, label),
    }
}

/// A foot window viewing the terminal at `addr`. It outlives this process
fn open(addr: &str, label: &str) {
    let runtime = env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let display = env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    let server = PathBuf::from(runtime).join(format!("foot-{display}.sock")).exists();
    let result = Command::new(if server { "footclient" } else { "foot" })
        .args(["--app-id", &format!("hc:{addr}"), "--title", label])
        .arg(exe())
        .args(["view", addr])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match result {
        // Waited on out of the way, since the bar opens windows too and lives on: one never waited on stays a
        // zombie under it
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(err) => eprintln!("lookout: can't open foot: {err}"),
    }
}

/// This program, to run again in a window. Rebuilt or reinstalled while running, Linux names it "<path> (deleted)",
/// which no window could start, so it's the new one at the same path, or failing that whichever is on PATH
fn exe() -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let path = env::current_exe().unwrap_or_default();
    let bytes = path.as_os_str().as_bytes();
    let path = Path::new(std::ffi::OsStr::from_bytes(bytes.strip_suffix(b" (deleted)").unwrap_or(bytes)));
    if path.is_file() { path.to_path_buf() } else { "lookout".into() }
}

/// Where the daemon writes the agents that want you and you haven't looked at, most first: address and label
pub fn seen_file() -> PathBuf {
    PathBuf::from(env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("lookout.wanting")
}

/// The daemon's pid while it's running. Only then can what it writes be trusted, and a stale pid could be anything,
/// even an agent window's `lookout view`, so it's only the daemon if it's lookout started with nothing after it
pub fn daemon() -> Option<i32> {
    let pid: i32 = fs::read_to_string(desktop::pid_file()).ok()?.trim().parse().ok()?;
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let mut args = cmdline.split(|&b| b == 0).filter(|a| !a.is_empty());
    let named = args.next().is_some_and(|exe| exe.rsplit(|&b| b == b'/').next() == Some(b"lookout"));
    (named && args.next().is_none()).then_some(pid)
}

fn seen_state() -> Option<Vec<(String, String)>> {
    // Only trust it while the daemon that writes it is running
    daemon()?;
    let text = fs::read_to_string(seen_file()).ok()?;
    Some(text.lines().filter_map(|l| l.split_once('\t')).map(|(a, l)| (a.to_string(), l.to_string())).collect())
}
