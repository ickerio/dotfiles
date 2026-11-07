//! What the Hyprland binds run: pick an agent, start one, jump to the one that wants you, end one, and the loop each
//! agent's foot window runs. An agent's window has the class "hc:<address>", which is how it's found again

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{exit, Child, Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

use crate::desktop;
use crate::hc::{self, Term};
use crate::hypr;
use crate::sys;
use crate::waiting;

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

/// Tells the daemon to peek (SIGUSR1) or stop (SIGUSR2)
pub fn signal(signal: i32) {
    let pid = fs::read_to_string(desktop::pid_file()).ok().and_then(|p| p.trim().parse::<i32>().ok());
    // Only if it's still lookout, a stale pid could be anything by now
    let Some(pid) = pid.filter(|pid| fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "lookout")) else {
        return;
    };
    let _ = sys::send_signal(pid, signal);
}

/// Every terminal in fuzzel: agents that want you first, so Enter goes straight to the one that wants you most, then
/// working ones, the rest, ones whose Claude can be resumed, and plain shells. Picking one focuses its window or opens
/// one, resuming its Claude first if it needs it
pub fn pick() {
    let mut terms = hc::list();
    // The daemon's order when it's running, since it knows which you've already looked at. Without it, hc's
    let wanting: Option<Vec<String>> = seen_state().map(|agents| agents.into_iter().map(|(addr, _)| addr).collect());
    // Both take a round trip to the other machines, so they go at the same time
    let labels = waiting_labels(&terms);
    let resumable = sessions(&terms);
    let waiting = waiting_from(labels);
    let rank = |t: &Term| {
        let wants = match &wanting {
            Some(wanting) => wanting.iter().position(|a| *a == t.addr),
            None if waiting.contains(&t.addr) => None,
            None => t.status.as_deref().and_then(hc::urgency).map(usize::from),
        };
        wants.map_or_else(|| rest(t, &resumable), |i| (0, i))
    };
    terms.sort_by_cached_key(|t| (rank(t), t.label().to_string()));

    let names = terms.iter().map(|t| t.label().len()).max().unwrap_or(0);
    let machines = terms.iter().map(|t| t.machine().unwrap_or(&hc::HOST).len()).max().unwrap_or(0);
    let mut entries: Vec<String> = terms
        .iter()
        .map(|t| {
            let wants = rank(t).0 == 0;
            let (mark, status) = match t.status.as_deref() {
                Some("blocked") => ('!', "blocked"),
                Some("error") => ('x', "error"),
                Some("done") if waiting.contains(&t.addr) => ('-', "wait"),
                Some("done") if wants => ('*', "done"),
                Some("working") => ('~', "working"),
                Some(status) => ('-', status),
                None if resumable.contains_key(&t.addr) => ('>', "resume"),
                None => (' ', ""),
            };
            let machine = t.machine().unwrap_or(&hc::HOST);
            format!("{mark} {:names$}  {status:7}  {machine:machines$}  {}", t.label(), hc::tilde(&t.cwd))
        })
        .collect();
    entries.push("+ new agent".to_string());
    match choose("agent", &entries) {
        Some(i) if i == terms.len() => new(),
        Some(i) => {
            let term = &terms[i];
            if let Some(id) = resumable.get(&term.addr) {
                resume(term, id);
            }
            focus_or_open(&term.addr, term.label());
        }
        None => {}
    }
}

/// Where a terminal that doesn't want you goes in the pick list
fn rest(t: &Term, resumable: &HashMap<String, String>) -> (u8, usize) {
    match t.status.as_deref() {
        Some("working") => (1, 0),
        Some(_) => (2, 0),
        None if resumable.contains_key(&t.addr) => (3, 0),
        None => (4, 0),
    }
}

/// Asks hc for the labels of every finished agent, since `hc ls` leaves them out. Started here, read by waiting_from
fn waiting_labels(terms: &[Term]) -> Vec<(String, Child)> {
    let done = terms.iter().filter(|t| t.status.as_deref() == Some("done"));
    done.filter_map(|t| {
        let mut get = hc::command(&["get", &t.addr, "--json"]);
        get.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        get.spawn().ok().map(|child| (t.addr.clone(), child))
    })
    .collect()
}

/// The finished agents still waiting on background tasks, which a Stop hook marks with a label
fn waiting_from(labels: Vec<(String, Child)>) -> HashSet<String> {
    labels
        .into_iter()
        .filter_map(|(addr, child)| {
            let term: Value = serde_json::from_slice(&child.wait_with_output().ok()?.stdout).ok()?;
            hc::str(&term["labels"][waiting::LABEL]).map(|_| addr)
        })
        .collect()
}

/// The Claude sessions that shells at their prompt can resume, by address: an agent that exited, or a kept terminal
/// that came back as a shell after a reboot. Found by the terminal's name, which `lookout new` gave its Claude
fn sessions(terms: &[Term]) -> HashMap<String, String> {
    let mut by_machine: HashMap<Option<&str>, Vec<&Term>> = HashMap::new();
    for term in terms.iter().filter(|t| t.at_prompt() && t.name.is_some()) {
        by_machine.entry(term.machine()).or_default().push(term);
    }
    // Every machine at once, since a remote one takes a round trip
    let running: Vec<_> = by_machine
        .iter()
        .filter_map(|(machine, terms)| {
            let args: Vec<&str> = terms.iter().flat_map(|t| [t.label(), t.cwd.as_str()]).collect();
            hc::sh(*machine, SESSIONS, &args).spawn().ok().map(|child| (terms, child))
        })
        .collect();
    let mut found = HashMap::new();
    for (terms, child) in running {
        let Ok(out) = child.wait_with_output() else { continue };
        for (name, id) in String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.split_once(' ')) {
            if let Some(term) = terms.iter().find(|t| t.label() == name) {
                found.insert(term.addr.clone(), id.to_string());
            }
        }
    }
    found
}

/// Types `claude --resume` into a terminal's shell, picking up its conversation where it left off
fn resume(term: &Term, id: &str) {
    let _ = hc::command(&["send", &term.addr, &format!("claude --resume {id}")]).stdout(Stdio::null()).status();
}

/// Picks a project on any machine, starts a shell there with Claude in it, and opens its window
pub fn new() {
    // Every machine at once, the other machines first since they're where agents mostly live
    let machines: Vec<Option<String>> = hc::machines().into_iter().map(Some).chain([None]).collect();
    let running: Vec<_> = machines.iter().map(|m| hc::sh(m.as_deref(), PROJECTS, &[]).spawn()).collect();
    let mut places: Vec<(Option<String>, String)> = Vec::new();
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
    let width = places.iter().map(|(m, _)| m.as_deref().unwrap_or(&hc::HOST).len()).max().unwrap_or(0);
    let entries: Vec<String> =
        places.iter().map(|(m, dir)| format!("{:width$}  {}", m.as_deref().unwrap_or(&hc::HOST), hc::tilde(dir))).collect();
    let Some(i) = choose("new agent in", &entries) else { return };
    let (machine, dir) = &places[i];

    // Named after its folder, made unique among what's running
    let base = term_name(&Path::new(dir).file_name().map_or(String::new(), |n| n.to_string_lossy().to_string()));
    let taken: Vec<String> = hc::list().iter().filter_map(|t| t.name.clone()).collect();
    let name = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|n| !taken.contains(n)).unwrap();

    // A shell rather than Claude directly: it loads the login's PATH, and it's still there if Claude exits. Kept, so
    // that after a reboot it comes back as a shell that `lookout pick` can resume Claude in
    let mut args = vec!["run", "--bg", "--keep", "--no-parent", "--cwd", dir, "--name", &name];
    if let Some(m) = machine {
        args.extend(["--on", m]);
    }
    let run = match hc::json(&args) {
        Ok(run) => run,
        Err(err) => return tell(&format!("can't start {name}: {err}")),
    };
    let Some(addr) = hc::str(&run["address"]) else { return };
    // Named, so the window's title and /resume show it, and the session can be found again by it
    let _ = hc::command(&["send", &addr, &format!("claude -n {name}")]).stdout(Stdio::null()).status();
    open(&addr, &name);
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

/// Ends the agent in the focused window, after asking
pub fn kill() {
    let Some(class) = hypr::focused_class() else { return };
    let Some(addr) = class.strip_prefix("hc:") else { return };
    let label = hc::list().into_iter().find(|t| t.addr == addr).map_or(addr.to_string(), |t| t.label().to_string());
    if choose(&format!("end {label}?"), &["no".into(), format!("yes, end {label}")]) == Some(1) {
        // An explicit kill is final even for a kept terminal
        if let Err(err) = hc::call(&["kill", addr]) {
            return tell(&format!("can't end {label}: {err}"));
        }
        let _ = hc::command(&["rm", addr]).stdout(Stdio::null()).stderr(Stdio::null()).status();
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

fn focus_or_open(addr: &str, label: &str) {
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
    let exe = env::current_exe().map_or("lookout".into(), |p| p.to_string_lossy().to_string());
    let result = Command::new(if server { "footclient" } else { "foot" })
        .args(["--app-id", &format!("hc:{addr}"), "--title", label, &exe, "view", addr])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(err) = result {
        eprintln!("lookout: can't open foot: {err}");
    }
}

/// Says what went wrong in fuzzel, since a bind's output goes nowhere
fn tell(msg: &str) {
    choose(&format!("lookout: {msg}"), &["ok".into()]);
}

/// Shows `entries` in fuzzel and returns the index picked, None if you escaped
fn choose(prompt: &str, entries: &[String]) -> Option<usize> {
    let lines = entries.len().clamp(1, 15).to_string();
    let mut fuzzel = Command::new("fuzzel")
        .args(["--dmenu", "--index", "--width", "80", "--lines", &lines, "--prompt", &format!("{prompt} > ")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| eprintln!("lookout: can't run fuzzel: {err}"))
        .ok()?;
    fuzzel.stdin.take()?.write_all(entries.join("\n").as_bytes()).ok()?;
    let out = fuzzel.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse::<i64>().ok().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < entries.len())
}

/// Where the daemon writes the agents that want you and you haven't looked at, most first: address and label
pub fn seen_file() -> PathBuf {
    PathBuf::from(env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("lookout.wanting")
}

fn seen_state() -> Option<Vec<(String, String)>> {
    // Only trust it while the daemon that writes it is running
    let pid = fs::read_to_string(desktop::pid_file()).ok()?;
    fs::read_to_string(format!("/proc/{}/comm", pid.trim())).ok().filter(|c| c.trim() == "lookout")?;
    let text = fs::read_to_string(seen_file()).ok()?;
    Some(text.lines().filter_map(|l| l.split_once('\t')).map(|(a, l)| (a.to_string(), l.to_string())).collect())
}
