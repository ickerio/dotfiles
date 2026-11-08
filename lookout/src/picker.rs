//! What the sheet lists, and the search over it. SUPER + A, or a click on the bar, shows every agent by what it wants
//! and the shells after them, with a line to type into: typing narrows them to the ones its letters appear in, in
//! order, best first, with the projects an agent could be started in after them. SUPER + SHIFT + A searches only the
//! projects, SUPER + SHIFT + Q asks before ending the focused agent, and what went wrong is said in a row

use std::collections::HashMap;

use crate::agents::Agent;
use crate::cmd::Detail;
use crate::hc;
use crate::ui::{self, Group, Icon};

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Every terminal, and the projects to start one in once something's typed
    Search,
    Projects,
    /// Asking before ending an agent
    End,
    /// Something went wrong, said in a row
    Note,
}

#[derive(Clone)]
pub enum Action {
    /// Go to a terminal's window, resuming its Claude session first if it has one to resume
    Go { addr: String, label: String, resume: Option<String> },
    /// Search only the projects, to start an agent in one
    New,
    Start { machine: Option<String>, dir: String },
    End { addr: String, label: String },
    Close,
}

impl Action {
    /// What Enter does, in a word or two
    pub fn verb(&self) -> &'static str {
        match self {
            Action::Go { resume: Some(_), .. } => "resume",
            Action::Go { .. } => "open",
            Action::New => "new agent",
            Action::Start { .. } => "start",
            Action::End { .. } => "end it",
            Action::Close => "close",
        }
    }
}

/// Which heading a row goes under
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Needs,
    Busy,
    Quiet,
    /// Terminals with no agent reporting in them
    Shells,
    /// Starting an agent
    New,
    /// A question's answers, or what went wrong
    Plain,
}

/// When a row is in the list at all
#[derive(Clone, Copy, PartialEq)]
pub enum Shows {
    Always,
    /// Only before anything's typed: "new agent", which typing a project's name replaces
    Untyped,
    /// Only once something's typed: the projects, which are too many to list unasked
    Typed,
}

pub struct Row {
    pub icon: Option<(Icon, u32)>,
    pub name: String,
    pub name_colour: u32,
    pub machine: String,
    pub state: String,
    pub state_colour: u32,
    /// What it says: an agent's message, or where a shell or project is
    pub detail: String,
    pub detail_colour: u32,
    /// Where it is, which typing matches too. Shown in place of the detail when that's what matched
    pub place: String,
    /// When it came into its state, ms since the epoch, 0 if unknown or it has none
    pub since: u64,
    pub section: Section,
    pub shows: Shows,
    /// The terminal whose window is focused, marked as in the bar
    pub current: bool,
    pub action: Action,
}

impl Row {
    fn blank(section: Section, action: Action) -> Row {
        Row {
            icon: None,
            name: String::new(),
            name_colour: ui::INK,
            machine: String::new(),
            state: String::new(),
            state_colour: ui::DIM,
            detail: String::new(),
            detail_colour: ui::DIM,
            place: String::new(),
            since: 0,
            section,
            shows: Shows::Always,
            current: false,
            action,
        }
    }

    /// Which row it is across new rows, whatever its state now says
    fn key(&self) -> String {
        match &self.action {
            Action::Go { addr, .. } | Action::End { addr, .. } => addr.clone(),
            Action::Start { machine, dir } => format!("{}:{dir}", machine.as_deref().unwrap_or("")),
            Action::New | Action::Close => self.name.clone(),
        }
    }
}

/// Which of a row's columns letters matched in, as `Match::field`
pub const NAME: u8 = 0;
pub const MACHINE: u8 = 1;
pub const PLACE: u8 = 2;

/// Where what's typed matched in a row: the column, and the character in it
pub type Match = Vec<(u8, usize)>;

pub struct Picker {
    pub mode: Mode,
    /// What it's asking, before what's typed: "new agent in", "end ronin?"
    pub prompt: String,
    pub query: String,
    pub rows: Vec<Row>,
    /// The rows in the list, in order, with where what's typed matched in each
    pub shown: Vec<(usize, Match)>,
    pub selected: usize,
    /// The first of `shown` on screen, when there are more than fit
    pub top: usize,
    /// Still finding more rows (another machine's projects)
    pub looking: bool,
}

impl Picker {
    pub fn new(mode: Mode, prompt: String, rows: Vec<Row>) -> Picker {
        let mut picker = Picker { mode, prompt, query: String::new(), rows, shown: Vec::new(), selected: 0, top: 0, looking: false };
        picker.filter();
        picker
    }

    /// Whether there's a line to type into
    pub fn types(&self) -> bool {
        matches!(self.mode, Mode::Search | Mode::Projects)
    }

    pub fn typed(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// New rows, e.g. once another machine has answered, keeping what's typed and staying on the same row, or
    /// where it was if it's gone
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let (on, was, top) = (self.selected_row().map(Row::key), self.selected, self.top);
        self.rows = rows;
        self.filter();
        if on.is_some() {
            let at = self.shown.iter().position(|(r, _)| Some(self.rows[*r].key()) == on).unwrap_or(was);
            self.top = top.min(self.shown.len().saturating_sub(ui::SHEET_ROWS));
            self.select(at);
        }
    }

    pub fn type_text(&mut self, text: &str) {
        if self.types() {
            self.query.push_str(text);
            self.filter();
        }
    }

    pub fn backspace(&mut self, word: bool) {
        if !self.types() {
            return;
        }
        if word {
            let kept = self.query.trim_end().rfind(' ').map_or(0, |i| i + 1);
            self.query.truncate(kept);
        } else {
            self.query.pop();
        }
        self.filter();
    }

    pub fn clear(&mut self) {
        if !self.types() {
            return;
        }
        self.query.clear();
        self.filter();
    }

    pub fn move_by(&mut self, by: i32) {
        if self.shown.is_empty() {
            return;
        }
        let last = self.shown.len() as i32 - 1;
        self.select((self.selected as i32 + by).clamp(0, last) as usize);
    }

    pub fn select(&mut self, i: usize) {
        self.selected = i.min(self.shown.len().saturating_sub(1));
        // Keep it on screen
        let rows = ui::SHEET_ROWS;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + rows {
            self.top = self.selected + 1 - rows;
        }
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.shown.get(self.selected).map(|(r, _)| &self.rows[*r])
    }

    /// The heading a row goes under, as the list stands: by what each agent wants until something's typed, then
    /// best match first
    pub fn heading(&self, row: &Row) -> (Section, Option<&'static str>) {
        // Typed, the projects are only set apart by a gap: their + already says what they're for
        if self.typed() || self.mode != Mode::Search {
            return (if row.section == Section::New { Section::New } else { Section::Plain }, None);
        }
        let title = match row.section {
            Section::Needs => Some("NEEDS YOU"),
            Section::Busy => Some("BUSY"),
            Section::Quiet => Some("QUIET"),
            Section::Shells => Some("SHELLS"),
            Section::New | Section::Plain => None,
        };
        (row.section, title)
    }

    fn filter(&mut self) {
        let words: Vec<Vec<char>> = self.query.split_whitespace().map(|w| w.chars().map(lower).collect()).collect();
        let typed = !words.is_empty();
        let mut shown: Vec<(bool, i32, usize, Match)> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match row.shows {
                Shows::Always => true,
                Shows::Untyped => !typed,
                Shows::Typed => typed,
            })
            .filter_map(|(i, row)| {
                // Every word typed, each somewhere in the row: "dot lap" is dotfiles on the laptop
                let mut found = (0, Vec::new());
                for word in &words {
                    let (score, at) = matches(word, row)?;
                    found.0 += score;
                    found.1.extend(at);
                }
                Some((row.section == Section::New, found.0, i, found.1))
            })
            .collect();
        // Best first, terminals before projects, and in the rows' own order when as good
        if typed {
            shown.sort_by_key(|(new, score, i, _)| (*new, -score, *i));
        }
        self.shown = shown.into_iter().map(|(.., i, at)| (i, at)).collect();
        self.top = 0;
        // Untyped, Enter goes to what wants you most rather than the window you're already in
        let first = (!typed).then(|| self.shown.iter().position(|(r, _)| !self.rows[*r].current)).flatten();
        self.select(first.unwrap_or(0));
    }
}

/// How well `query` matches a row, and where: within its name, machine or place, whichever matches best, the name
/// counting most. Never across them, nor its state, which says nothing about which row it is
fn matches(query: &[char], row: &Row) -> Option<(i32, Match)> {
    [(NAME, &row.name), (MACHINE, &row.machine), (PLACE, &row.place)]
        .into_iter()
        .filter_map(|(field, hay)| fuzzy(query, hay, field == NAME).map(|(score, at)| (score, at.into_iter().map(|i| (field, i)).collect())))
        .max_by_key(|(score, _)| *score)
}

/// Whether `query`'s letters all appear in `hay` in order, and how well: runs of letters together and letters that
/// start words count most, and all of them more in a `name`. Where in `hay` (by character) each matched
fn fuzzy(query: &[char], hay: &str, name: bool) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    // A character at a time, so positions stay those of `hay` even where lowercasing makes one character two
    let hay: Vec<char> = hay.chars().map(lower).collect();
    // Starting from each place the first letter appears, matching each next letter as soon as it can, keep the best
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in (0..hay.len()).filter(|&i| hay[i] == query[0]) {
        let mut at = vec![start];
        for &q in &query[1..] {
            let from = at.last().unwrap() + 1;
            match hay[from..].iter().position(|&h| h == q) {
                Some(i) => at.push(from + i),
                None => break,
            }
        }
        if at.len() < query.len() {
            break;
        }
        let score = at
            .iter()
            .enumerate()
            .map(|(n, &i)| {
                let together = n > 0 && at[n - 1] + 1 == i;
                let word_start = i == 0 || !hay[i - 1].is_alphanumeric();
                1 + if together { 6 } else { 0 } + if word_start { 8 } else { 0 } + if name { 6 } else { 0 }
            })
            .sum::<i32>()
            - (at.last().unwrap() - start) as i32 / 4
            - start as i32 / 16;
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, at));
        }
    }
    best
}

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The agents as rows, in the bar's order grouped by what they want
pub fn agent_rows(agents: &[Agent], focused: Option<&str>, cwd: &dyn Fn(&str) -> String) -> Vec<Row> {
    let mut rows: Vec<Row> = agents
        .iter()
        .map(|agent| {
            let look = ui::look(agent);
            let needs = look.group == Group::Needs;
            let current = focused == Some(agent.addr.as_str());
            let place = hc::tilde(&cwd(&agent.addr));
            let said = ui::message(agent);
            Row {
                icon: Some((look.icon, look.accent)),
                name: agent.label().to_string(),
                name_colour: if needs || current { ui::INK } else { look.name },
                machine: agent.device().to_string(),
                state: look.word.to_string(),
                state_colour: match look.icon {
                    Icon::Ask | Icon::Error => look.accent,
                    _ if needs => ui::MID,
                    _ => look.name,
                },
                detail_colour: if needs { ui::SAYS } else { look.name },
                detail: said.unwrap_or_default(),
                place,
                since: agent.since,
                section: match look.group {
                    Group::Needs => Section::Needs,
                    Group::Busy => Section::Busy,
                    Group::Quiet => Section::Quiet,
                },
                shows: Shows::Always,
                current,
                action: Action::Go { addr: agent.addr.clone(), label: agent.label().to_string(), resume: None },
            }
        })
        .collect();
    rows.sort_by_key(|r| r.section);
    rows
}

/// The terminals with no agent reporting: shells whose Claude can be resumed, ones running a Claude that hasn't said
/// what it's doing, then plain shells
pub fn shell_rows(others: &[hc::Term], focused: Option<&str>, details: &HashMap<String, Detail>, resumable: &HashMap<String, String>) -> Vec<Row> {
    let kind = |t: &hc::Term| {
        let claude = details.get(&t.addr).is_some_and(|d| d.foreground.as_deref() == Some("claude"));
        match (resumable.get(&t.addr), claude) {
            (Some(_), _) => 0,
            (None, true) => 1,
            _ => 2,
        }
    };
    let mut others: Vec<&hc::Term> = others.iter().collect();
    others.sort_by_cached_key(|t| (kind(t), t.label().to_string()));
    others
        .into_iter()
        .map(|t| {
            let state = match kind(t) {
                0 => "ended",
                1 => "claude",
                _ => "shell",
            };
            let place = hc::tilde(&t.cwd);
            // A shell with no name by its folder, rather than an address that repeats its machine
            let name = t.name.clone().unwrap_or_else(|| match place.as_str() {
                "~" => "home".to_string(),
                _ => t.cwd.rsplit('/').next().unwrap_or(&t.addr).to_string(),
            });
            let resume = resumable.get(&t.addr).cloned();
            Row {
                icon: Some((Icon::Shell, ui::DIM)),
                name,
                name_colour: if resume.is_some() { ui::MID } else { ui::DIM },
                machine: t.machine().unwrap_or(&hc::HOST).to_string(),
                state: state.to_string(),
                // Home needs no "~" after it
                detail: if resume.is_some() { "resumable".into() } else if place == "~" { String::new() } else { place.clone() },
                place,
                current: focused == Some(t.addr.as_str()),
                action: Action::Go { addr: t.addr.clone(), label: t.label().to_string(), resume },
                ..Row::blank(Section::Shells, Action::Close)
            }
        })
        .collect()
}

/// The row that starts an agent, before anything's typed
pub fn new_row() -> Row {
    Row {
        icon: Some((Icon::New, ui::MID)),
        name: "new agent".into(),
        name_colour: ui::MID,
        detail: "type to pick a project".into(),
        shows: Shows::Untyped,
        ..Row::blank(Section::New, Action::New)
    }
}

/// Every project on every machine, as rows. `shows` is Typed among the terminals, Always on their own
pub fn project_rows(projects: &[(Option<String>, String)], shows: Shows) -> Vec<Row> {
    projects
        .iter()
        .map(|(machine, dir)| {
            let place = hc::tilde(dir);
            let name = if place == "~" { "home".to_string() } else { dir.rsplit('/').next().unwrap_or(dir).to_string() };
            Row {
                icon: Some((Icon::New, ui::DIM)),
                name,
                name_colour: ui::INK,
                machine: machine.clone().unwrap_or_else(|| hc::HOST.clone()),
                // Among the terminals, what it'd be; on their own they're all new
                state: if shows == Shows::Typed { "new".into() } else { String::new() },
                detail: place.clone(),
                place,
                shows,
                ..Row::blank(Section::New, Action::Start { machine: machine.clone(), dir: dir.clone() })
            }
        })
        .collect()
}

/// Asking before ending an agent: keeping it is what Enter does unless you move to ending it
pub fn end(addr: &str, label: &str, machine: &str) -> Picker {
    let rows = vec![
        Row { name: "keep it".into(), ..Row::blank(Section::Plain, Action::Close) },
        Row {
            name: "end it".into(),
            name_colour: ui::RED,
            detail: "closes its terminal too".into(),
            ..Row::blank(Section::Plain, Action::End { addr: addr.to_string(), label: label.to_string() })
        },
    ];
    Picker::new(Mode::End, format!("end {label} on {machine}?"), rows)
}

/// What went wrong: what couldn't be done above, and why in the row, "can't start web" and "hc: no such machine"
pub fn note(msg: &str) -> Picker {
    let (what, why) = msg.split_once(": ").unwrap_or(("something went wrong", msg));
    let row = Row { name: why.to_string(), ..Row::blank(Section::Plain, Action::Close) };
    Picker::new(Mode::Note, what.to_string(), vec![row])
}

/// What SUPER + A searches: the agents by what they want, then the shells, then starting an agent, by its row
/// or in a project typed
pub fn search_rows(
    agents: &[Agent],
    others: &[hc::Term],
    focused: Option<&str>,
    cwd: &dyn Fn(&str) -> String,
    details: &HashMap<String, Detail>,
    resumable: &HashMap<String, String>,
    projects: &[(Option<String>, String)],
) -> Vec<Row> {
    let mut rows = agent_rows(agents, focused, cwd);
    rows.extend(shell_rows(others, focused, details, resumable));
    rows.push(new_row());
    rows.extend(project_rows(projects, Shows::Typed));
    rows
}
