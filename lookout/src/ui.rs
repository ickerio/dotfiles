//! What lookout looks like, drawn the same way live and by `lookout shot`:
//!
//! - the bar across the top of the screen: every agent as a tab in the order their windows are on screen, grouped
//!   under each workspace's number with a line between, then the ones with no window with no number. The ones that
//!   want you are tinted pills, and the stats and clock are on the right. What they want is in the sheet
//! - the sheet that hangs from its middle on SUPER + A or a click: every agent in detail, with a line to type into to
//!   go to any terminal or start an agent (see picker.rs)
//! - a 2px line across the top when a fullscreen window hides the bar, a stretch of it for each agent that wants
//!   you in its colour
//!
//! Every state has its own icon shape as well as its own colour, so they tell apart at a glance:
//! ask ◆  error ✕  done ●  done and seen ✓  working ▶  waiting ⧗  idle –, and a shell's prompt >_. Each is a still sign of its state rather
//! than a picture of motion, since nothing moves. In the bar, agents that ask nothing of you (idle, or done and seen)
//! have no icon at all: their dim name is enough
//!
//! Nothing animates, so nothing wakes up to redraw: it changes when the agents or the minute do

use std::collections::HashMap;

use crate::agents::Agent;
use crate::draw::{opaque, width, Canvas};
use crate::font::Size;
use crate::hypr::Place;
use crate::picker::{Action, Match, Mode, Picker, Section, MACHINE, NAME, PLACE};
use crate::stats::Item;

pub const BAR_H: i32 = 32;
/// The sheet is this wide, and tall enough for SHEET_ROWS agents
pub const SHEET_W: i32 = 760;
pub const SHEET_ROWS: usize = 20;

/// Room at each end of the bar, and between the pills and the bar's edges
const PAD: i32 = 12;
const PILL_H: i32 = 20;
const PILL_PAD: i32 = 8;
const TAB_GAP: i32 = 4;
/// Icons fit a box this big, with this gap before their label
const ICON: i32 = 10;
const ICON_GAP: i32 = 7;
/// Between the runs inside a tab (name, age) and between the stats
const SPACE: i32 = 8;
const STAT_GAP: i32 = 18;
/// Before the hint that shows when there are no agents
const WORKSPACE_GAP: i32 = 24;
/// Each side of the line between workspaces, its height, and between a workspace's number and its first tab
const DIVIDER_GAP: i32 = 9;
const DIVIDER_H: i32 = 14;
const NUM_GAP: i32 = 4;
/// Inside a tab that's only an icon, so it's as wide as a pill is tall
const ICON_PAD: i32 = 5;
const NAME_COLS: usize = 16;
/// Pills show how long they've been waiting from this many seconds, and it turns bright from STALE
const SHOW_AGE: u64 = 180;
const STALE: u64 = 600;
/// Room inside the sheet's edges, the same as a terminal's padding
const SHEET_PAD: i32 = 12;
const ROW_H: i32 = 28;
/// A section heading's row, and the space before each one after the first
const HEAD_H: i32 = 20;
const GROUP_GAP: i32 = 10;
/// Square, like the windows
const PILL_R: f32 = 0.0;
/// Where the sheet hangs: flush under the bar, centred on the screen (it's anchored to the top only, so the
/// compositor centres it), where a launcher is looked for. The bar opens in the middle rather than another window
/// appearing, and its three open edges are alike
pub const SHEET_Y: i32 = BAR_H;
/// The sheet's surface, the grey black stands for on it (see Canvas::raise): 0 is the bar's own black, so the two
/// are one shape. A step up the ramp (0x1d) would show it raised above the windows instead
const SHEET_RAISE: u32 = 0;

// Colours as 0xRRGGBB: three greys for how much something matters, amber for asking and red for errors, and dark
// tints of them behind the pills
pub const INK: u32 = 0xe6e6e6;
pub const MID: u32 = 0x8a8a8a;
pub const DIM: u32 = 0x5a5a5a;
const RULE: u32 = 0x262626;
const AMBER: u32 = 0xf0b44c;
pub const RED: u32 = 0xf05545;
const AMBER_TINT: u32 = 0x3d2c0e;
const RED_TINT: u32 = 0x2e1714;
const DONE_TINT: u32 = 0x1a1d22;
const HOVER: u32 = 0x161616;
const COUNT: u32 = 0x3a3a3a;
/// What an agent that wants you says, a step under the name
pub const SAYS: u32 = 0xb4b4b4;
/// Under a tab the pointer's over
const HOVER_RULE: u32 = 0x3a3a3a;
const DIVIDER: u32 = 0x3a3a3a;

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Ask,
    Error,
    Done,
    Seen,
    Working,
    Waiting,
    Idle,
    /// Something to start
    New,
    /// The line to type into
    Search,
    /// A terminal with no agent in it
    Shell,
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub enum Group {
    /// Asking, failed, or finished and not looked at yet
    Needs,
    Busy,
    Quiet,
}

/// How an agent shows
pub struct Look {
    pub icon: Icon,
    pub group: Group,
    /// The icon's and state word's colour
    pub accent: u32,
    /// Behind its pill, None for a plain tab
    pub tint: Option<u32>,
    pub name: u32,
    /// Its state in a word
    pub word: &'static str,
}

pub fn look(agent: &Agent) -> Look {
    let wants = agent.urgency().is_some();
    let (icon, group, accent, tint, name, word) = match agent.state.as_str() {
        "blocked" if wants => (Icon::Ask, Group::Needs, AMBER, Some(AMBER_TINT), INK, "asks"),
        // Looked at, but still stuck until you answer
        "blocked" => (Icon::Ask, Group::Busy, AMBER, None, MID, "asks"),
        "error" if wants => (Icon::Error, Group::Needs, RED, Some(RED_TINT), INK, "error"),
        "error" => (Icon::Error, Group::Quiet, RED, None, DIM, "error"),
        "done" if wants => (Icon::Done, Group::Needs, INK, Some(DONE_TINT), INK, "done"),
        "done" => (Icon::Seen, Group::Quiet, DIM, None, DIM, "done"),
        "working" => (Icon::Working, Group::Busy, MID, None, MID, "working"),
        "waiting" => (Icon::Waiting, Group::Busy, MID, None, MID, "waiting"),
        _ => (Icon::Idle, Group::Quiet, DIM, None, DIM, "idle"),
    };
    Look { icon, group, accent, tint, name, word }
}

/// The colours of the line shown over a fullscreen window, one for each agent that wants you, most first. Empty if
/// none does
pub fn line_colours(agents: &[Agent]) -> Vec<u32> {
    agents.iter().filter(|a| look(a).group == Group::Needs).map(|a| look(a).accent).collect()
}

/// Everything there is to show
pub struct View<'a> {
    /// The bar's tabs, ordered as `by_place` leaves them
    pub agents: &'a [Agent],
    /// Address of the agent whose window is focused
    pub focused: Option<&'a str>,
    pub stats: &'a [Item],
    /// "14:32" and "Sat 7 Nov"
    pub clock: (&'a str, &'a str),
    /// Where each agent's window is, by address
    pub places: &'a HashMap<String, Place>,
    /// The name of the workspace showing, "2"
    pub workspace: &'a str,
    pub now_ms: u64,
    /// Whether an agent just started wanting you, so its pill shows brighter for a moment
    pub flash: &'a dyn Fn(&str) -> bool,
    /// Where the pointer is over the surface being drawn
    pub pointer: Option<(i32, i32)>,
}

/// What a click on part of the bar or sheet does
#[derive(Clone)]
pub enum Hit {
    /// What a row or an agent's tab does
    Do(Action),
    /// Open or close the sheet
    Sheet,
    /// A row of the sheet's, by its place in the list, and what it does
    Row(usize, Action),
    /// Go to this workspace
    Workspace(i64),
}

/// The order for the sheet and `lookout next`: the ones that want you first, most first and then longest waiting,
/// then the rest by name
pub fn order(agents: &mut [Agent]) {
    agents.sort_by(|a, b| {
        let key = |a: &Agent| (a.urgency().unwrap_or(u8::MAX), if a.urgency().is_some() { a.since } else { 0 });
        key(a).cmp(&key(b)).then_with(|| a.label().cmp(b.label())).then_with(|| a.addr.cmp(&b.addr))
    });
}

/// The bar's order: as their windows are on screen, so tabs only move when windows do. Agents with no window go last
pub fn by_place(agents: &mut [Agent], places: &HashMap<String, Place>) {
    let place = |a: &Agent| places.get(&a.addr).copied().unwrap_or((i64::MAX, 0, 0));
    agents.sort_by(|a, b| place(a).cmp(&place(b)).then_with(|| a.label().cmp(b.label())).then_with(|| a.addr.cmp(&b.addr)));
}

/// "40s", "3m", "2h", "4d"
pub fn age(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// How long it's been in its state, None if that isn't known
fn secs(view: &View, agent: &Agent) -> Option<u64> {
    (agent.since > 0).then(|| view.now_ms.saturating_sub(agent.since) / 1000)
}

/// What an agent says about itself, or about what it's waiting on
pub fn message(agent: &Agent) -> Option<String> {
    if agent.state == "waiting" {
        let s = if agent.tasks == 1 { "" } else { "s" };
        return Some(format!("{} background task{s}", agent.tasks));
    }
    let msg = agent.msg.clone().or_else(|| agent.kind.clone()).filter(|m| !m.trim().is_empty())?;
    let msg = msg.split(' ').map(short_path).collect::<Vec<_>>().join(" ");
    // Lower case like the rest, "Reading" as "reading", though not "API"
    let mut chars = msg.chars();
    Some(match (chars.next(), chars.next()) {
        (Some(first), Some(second)) if first.is_uppercase() && !second.is_uppercase() => first.to_lowercase().chain(msg.chars().skip(1)).collect(),
        _ => msg,
    })
}

/// A long path down to its last two parts, so what's cut off is where it is rather than what it is:
/// "/home/harry/dotfiles/lookout/src/desktop.rs" is "…/src/desktop.rs"
fn short_path(word: &str) -> String {
    let parts: Vec<&str> = word.split('/').collect();
    if word.len() <= 24 || parts.len() <= 3 {
        return word.to_string();
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}

/// `c` as the font has it: a space for any whitespace, "?" for what it lacks
fn printable(c: char) -> char {
    if c.is_ascii_graphic() || c == ' ' || c == '…' { c } else if c.is_whitespace() { ' ' } else { '?' }
}

/// `s` as printable ASCII on one line, cut to `cols` characters with "…" when longer
pub fn cut(s: &str, cols: usize) -> String {
    let ascii: String = s.chars().map(printable).collect();
    if ascii.chars().count() <= cols {
        return ascii;
    }
    if cols < 2 {
        return ascii.chars().take(cols).collect();
    }
    let cut: String = ascii.chars().take(cols - 1).collect();
    // End at the last whole word rather than a scrap of one, unless that loses too much
    let cut = match cut.rsplit_once(' ') {
        Some((before, _)) if before.trim_end().len() * 3 >= cols * 2 => before.to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end())
}

/// Text cells sit so the digits are centred on `mid`
fn text_top(size: Size, mid: i32) -> i32 {
    let f = size.font();
    mid - (f.top + f.baseline) / 2
}

/// Where the top of an icon goes on a line of text centred on `mid`: a pixel low, since the text beside it is mostly
/// lower case, which sits lower than the digits it's centred by
fn icon_top(mid: i32) -> i32 {
    mid - ICON / 2 + 1
}

impl Canvas<'_> {
    /// An icon in its ICON box with its top left at (x, y)
    pub fn icon(&mut self, icon: Icon, x: i32, y: i32, colour: u32) {
        let (cx, cy) = (x as f32 + ICON as f32 / 2.0, y as f32 + ICON as f32 / 2.0);
        let (bx, by, b) = (x - 1, y - 1, ICON + 2);
        let d = |px: f32, py: f32| ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
        // How far (px, py) is from the line from (ax, ay) to (bx, by), offsets from the centre
        let seg = |px: f32, py: f32, (ax, ay): (f32, f32), (bx, by): (f32, f32)| {
            let (ax, ay, bx, by) = (cx + ax, cy + ay, cx + bx, cy + by);
            let (vx, vy, wx, wy) = (bx - ax, by - ay, px - ax, py - ay);
            let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
            ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt()
        };
        match icon {
            Icon::Ask => self.shape(bx, by, b, b, colour, |px, py| (px - cx).abs() + (py - cy).abs() <= 5.2),
            Icon::Error => self.shape(bx, by, b, b, colour, |px, py| {
                let (dx, dy) = (px - cx, py - cy);
                dx.abs() <= 4.2 && dy.abs() <= 4.2 && ((dx - dy).abs() <= 1.6 || (dx + dy).abs() <= 1.6)
            }),
            Icon::Done => self.shape(bx, by, b, b, colour, |px, py| d(px, py) <= 4.3),
            // A tick: done, and you've seen it
            Icon::Seen => self.shape(bx, by, b, b, colour, |px, py| {
                seg(px, py, (-4.0, 0.2), (-1.3, 3.0)).min(seg(px, py, (-1.3, 3.0), (4.0, -3.2))) <= 0.8
            }),
            // A short dash: nothing going on
            Icon::Idle => self.shape(bx, by, b, b, colour, |px, py| (px - cx).abs() <= 2.6 && (py - cy).abs() <= 0.7),
            // A prompt, ">_"
            Icon::Shell => self.shape(bx, by, b, b, colour, |px, py| {
                let chevron = seg(px, py, (-4.4, -3.6), (-0.8, 0.0)).min(seg(px, py, (-0.8, 0.0), (-4.4, 3.6)));
                chevron.min(seg(px, py, (0.8, 3.8), (4.6, 3.8))) <= 0.85
            }),
            // Running, as a play sign: still, so it can't be mistaken for a spinner that's stuck
            Icon::Working => self.shape(bx, by, b, b, colour, |px, py| {
                let (dx, dy) = (px - cx + 0.6, py - cy);
                dx >= -3.4 && dy.abs() <= (4.4 - dx) * 0.58
            }),
            Icon::Search => self.shape(bx, by, b, b, colour, |px, py| {
                // A lens, and its handle out to the bottom right
                let (lx, ly) = (cx - 1.0, cy - 1.0);
                let ring = (((px - lx).powi(2) + (py - ly).powi(2)).sqrt() - 3.3).abs() <= 0.75;
                let t = ((px - lx) + (py - ly)) / 2.0;
                let handle = (3.2..=6.0).contains(&t) && ((px - lx) - (py - ly)).abs() <= 1.5;
                ring || handle
            }),
            Icon::New => self.shape(bx, by, b, b, colour, |px, py| {
                let (dx, dy) = ((px - cx).abs(), (py - cy).abs());
                (dx <= 0.8 && dy <= 4.0) || (dy <= 0.8 && dx <= 4.0)
            }),
            Icon::Waiting => self.shape(bx, by, b, b, colour, |px, py| {
                // An hourglass: a bar top and bottom, and two triangles meeting in the middle
                let (dx, dy) = (px - cx, py - cy);
                (dy.abs() <= 4.6 && dy.abs() >= 3.4 && dx.abs() <= 4.0) || (dy.abs() < 3.4 && dx.abs() <= dy.abs() * 0.95 + 0.5)
            }),
        }
    }

    /// `s` with `track` extra pixels after each character
    fn spaced(&mut self, x: i32, y: i32, size: Size, s: &str, colour: u32, track: i32) -> i32 {
        let mut x = x;
        for c in s.chars() {
            x = self.text(x, y, size, &c.to_string(), colour) + track;
        }
        x
    }

    /// `colour` blended `t` of the way towards `to`
    fn mix(colour: u32, to: u32, t: f32) -> u32 {
        let t = t.clamp(0.0, 1.0);
        let ch = |shift: u32| {
            let (a, b) = ((colour >> shift & 0xff) as f32, (to >> shift & 0xff) as f32);
            ((a + (b - a) * t).round() as u32) << shift
        };
        ch(16) | ch(8) | ch(0)
    }
}

/// How much detail the bar's tabs go into, less and less until they fit. What asks nothing of you goes first
#[derive(Clone, Copy)]
struct Detail {
    /// Quiet agents left to the sheet, and busy ones only an icon. The focused one stays as it is
    fold_quiet: bool,
    fold_busy: bool,
}

const DETAILS: [Detail; 3] = [
    Detail { fold_quiet: false, fold_busy: false },
    Detail { fold_quiet: true, fold_busy: false },
    Detail { fold_quiet: true, fold_busy: true },
];

/// One tab: maybe an icon, and runs of text, maybe on a pill
struct Tab {
    icon: Option<(Icon, u32)>,
    tint: Option<u32>,
    runs: Vec<(String, u32)>,
    /// Pixels the icon and name take, for the focus underline
    name_w: i32,
    focused: bool,
    hit: Hit,
}

impl Tab {
    fn pad(&self) -> i32 {
        if self.runs.is_empty() { ICON_PAD } else { PILL_PAD }
    }

    fn width(&self) -> i32 {
        if self.runs.is_empty() {
            return ICON + ICON_PAD * 2;
        }
        let runs: i32 = self.runs.iter().map(|(s, _)| width(Size::Ui, s)).sum::<i32>() + SPACE * (self.runs.len() as i32 - 1);
        PILL_PAD * 2 + self.icon.map_or(0, |_| ICON + ICON_GAP) + runs
    }
}

/// The tabs of one workspace, whose number leads them, or of the agents with no window, when `ws` is None
struct TabGroup {
    ws: Option<i64>,
    tabs: Vec<Tab>,
}

impl TabGroup {
    /// The space after the line before it. A plain tab with no number in front brings its own
    fn after_divider(&self) -> i32 {
        if self.ws.is_none() && self.tabs.first().is_some_and(|t| t.tint.is_none()) { 1 } else { DIVIDER_GAP }
    }

    fn width(&self) -> i32 {
        let number = self.ws.map_or(0, |ws| width(Size::Ui, &ws.to_string()));
        let gap = if self.ws.is_some() && !self.tabs.is_empty() { NUM_GAP } else { 0 };
        number + gap + tabs_width(&self.tabs)
    }
}

fn agent_tab(view: &View, agent: &Agent) -> Tab {
    let look = look(agent);
    let name = cut(agent.label(), NAME_COLS);
    // Quiet agents need no icon, their dim name says it
    let icon = (look.group != Group::Quiet).then_some((look.icon, look.accent));
    let name_w = icon.map_or(0, |_| ICON + ICON_GAP) + width(Size::Ui, &name);
    let mut runs = vec![(name, look.name)];
    if let Some(secs) = secs(view, agent).filter(|s| look.group == Group::Needs && *s >= SHOW_AGE) {
        runs.push((age(secs), if secs >= STALE { INK } else { DIM }));
    }
    // Just started wanting you: the pill brighter, its words white so they still read on it
    let flash = (view.flash)(&agent.addr);
    let tint = look.tint.map(|t| Canvas::mix(t, look.accent, if flash { 0.3 } else { 0.0 }));
    if flash {
        runs.iter_mut().for_each(|run| run.1 = INK);
    }
    Tab {
        icon,
        tint,
        runs,
        name_w,
        focused: view.focused == Some(agent.addr.as_str()),
        hit: Hit::Do(Action::Go { addr: agent.addr.clone(), label: agent.label().to_string(), resume: None }),
    }
}

/// The agents as `view` has them ordered, a group for each workspace. The workspace showing has one even with nothing
/// on it, so you can always see where you are
fn groups(view: &View, detail: Detail) -> Vec<TabGroup> {
    let mut groups: Vec<TabGroup> = Vec::new();
    for agent in view.agents {
        let ws = view.places.get(&agent.addr).map(|place| place.0);
        if groups.last().is_none_or(|g| g.ws != ws) {
            groups.push(TabGroup { ws, tabs: Vec::new() });
        }
        let focused = view.focused == Some(agent.addr.as_str());
        let tab = match look(agent).group {
            Group::Quiet if detail.fold_quiet && !focused => continue,
            Group::Busy if detail.fold_busy && !focused => Tab { runs: Vec::new(), ..agent_tab(view, agent) },
            _ => agent_tab(view, agent),
        };
        groups.last_mut().unwrap().tabs.push(tab);
    }
    groups.retain(|g| g.ws.is_some() || !g.tabs.is_empty());
    if let Some(current) = current_workspace(view).filter(|c| !groups.iter().any(|g| g.ws == Some(*c))) {
        let at = groups.iter().position(|g| g.ws.is_none_or(|ws| ws > current)).unwrap_or(groups.len());
        groups.insert(at, TabGroup { ws: Some(current), tabs: Vec::new() });
    }
    groups
}

fn current_workspace(view: &View) -> Option<i64> {
    view.workspace.parse().ok()
}

/// Takes a tab out to make room: the last that doesn't want you, if there is one
fn drop_tab(groups: &mut [TabGroup]) {
    let spots = || groups.iter().enumerate().flat_map(|(g, group)| group.tabs.iter().enumerate().map(move |(t, tab)| (g, t, tab.tint.is_none())));
    if let Some((g, t, _)) = spots().filter(|spot| spot.2).last().or(spots().last()) {
        groups[g].tabs.remove(t);
    }
}

fn tab_count(groups: &[TabGroup]) -> usize {
    groups.iter().map(|g| g.tabs.len()).sum()
}

fn tabs_width(tabs: &[Tab]) -> i32 {
    (tabs.iter().map(|t| t.width() + TAB_GAP).sum::<i32>() - TAB_GAP).max(0)
}

fn groups_width(groups: &[TabGroup]) -> i32 {
    groups.iter().enumerate().map(|(i, g)| g.width() + if i > 0 { DIVIDER_GAP + 1 + g.after_divider() } else { 0 }).sum()
}

/// Draws the bar, returning which spans of it do what when clicked
pub fn bar(c: &mut Canvas, view: &View) -> Vec<(i32, i32, Hit)> {
    // Short of room, the stats give way to the agents, all but the battery
    let all = bar_with(c, view, view.stats, false);
    if let Some(hits) = all {
        return hits;
    }
    let battery: Vec<Item> = view.stats.iter().filter(|s| s.0 == "BAT" || s.0 == "CHG").cloned().collect();
    bar_with(c, view, &battery, true).unwrap_or_default()
}

/// The bar with these stats, None if its agents only fit by leaving some out and `last` isn't set
fn bar_with(c: &mut Canvas, view: &View, stats: &[Item], last: bool) -> Option<Vec<(i32, i32, Hit)>> {
    c.fill(0, 0, c.w, c.h, opaque(0));
    let mut hits = Vec::new();
    let mid = BAR_H / 2;
    let ty = text_top(Size::Ui, mid);

    // The clock and stats from the right edge in
    let (time, date) = view.clock;
    let mut x = c.w - PAD - width(Size::Ui, time);
    let right_end = c.w - PAD;
    c.text(x, ty, Size::Ui, time, INK);
    x -= SPACE + width(Size::Ui, date);
    c.text(x, ty, Size::Ui, date, MID);
    for (label, value, note) in stats.iter().rev() {
        let note = note.as_deref().unwrap_or("");
        let w = width(Size::Ui, label) + 6 + width(Size::Ui, value) + width(Size::Ui, note);
        x -= STAT_GAP + w;
        let low = (*label == "BAT") && value.parse::<u32>().is_ok_and(|v| v <= 15);
        let vx = c.text(x, ty, Size::Ui, label, DIM) + 6;
        let nx = c.text(vx, ty, Size::Ui, value, if low { AMBER } else { MID });
        c.text(nx, ty, Size::Ui, note, DIM);
    }
    hits.push((x - STAT_GAP / 2, right_end + PAD, Hit::Sheet));
    let room = x - STAT_GAP - PAD;

    // The agents, in as much detail as fits
    let mut groups = Vec::new();
    for detail in DETAILS {
        groups = self::groups(view, detail);
        if PAD + groups_width(&groups) <= room {
            break;
        }
    }
    // Still too many: some of them become "+N", with room kept for it and a line before it
    let more = |n: usize| Tab { icon: None, tint: None, runs: vec![(format!("+{n}"), MID)], name_w: 0, focused: false, hit: Hit::Sheet };
    let mut hidden = 0;
    while tab_count(&groups) > 1 && PAD + groups_width(&groups) > room {
        drop_tab(&mut groups);
        hidden += 1;
        while tab_count(&groups) > 1 && PAD + groups_width(&groups) + DIVIDER_GAP * 2 + 1 + more(hidden).width() > room {
            drop_tab(&mut groups);
            hidden += 1;
        }
    }
    if hidden > 0 {
        if !last && stats.len() > 1 {
            return None;
        }
        if groups.last().is_none_or(|g| g.ws.is_some()) {
            groups.push(TabGroup { ws: None, tabs: Vec::new() });
        }
        groups.last_mut().unwrap().tabs.push(more(hidden));
    }
    let mut x = PAD;
    let pill_y = (BAR_H - PILL_H) / 2;
    let current = current_workspace(view);
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            x += DIVIDER_GAP;
            c.fill(x, (BAR_H - DIVIDER_H) / 2, 1, DIVIDER_H, opaque(DIVIDER));
            x += 1 + group.after_divider();
        }
        if let Some(ws) = group.ws {
            let end = c.text(x, ty, Size::Ui, &ws.to_string(), if Some(ws) == current { INK } else { DIM });
            hits.push((x - NUM_GAP, end + NUM_GAP / 2, Hit::Workspace(ws)));
            x = end + if group.tabs.is_empty() { 0 } else { NUM_GAP };
        }
        for tab in &group.tabs {
            let w = tab.width();
            let hovered = view.pointer.is_some_and(|(px, _)| px >= x && px < x + w);
            // A pill brightens under the pointer, and a plain tab gets a faint underline where focus has its bright one
            match tab.tint {
                Some(tint) => c.round_rect(x, pill_y, w, PILL_H, PILL_R, if hovered { Canvas::mix(tint, INK, 0.08) } else { tint }),
                None if hovered && !tab.focused => c.fill(x + tab.pad(), BAR_H - 2, w - 2 * tab.pad(), 2, opaque(HOVER_RULE)),
                None => {}
            }
            let ix = x + tab.pad();
            let mut tx = ix;
            if let Some((icon, colour)) = tab.icon {
                c.icon(icon, ix, icon_top(mid), colour);
                tx += ICON + ICON_GAP;
            }
            for (s, colour) in &tab.runs {
                tx = c.text(tx, ty, Size::Ui, s, if tab.focused && tab.tint.is_none() { INK } else { *colour }) + SPACE;
            }
            if tab.focused {
                c.fill(ix, BAR_H - 2, tab.name_w, 2, opaque(INK));
            }
            hits.push((x - TAB_GAP / 2, x + w + TAB_GAP / 2, tab.hit.clone()));
            x += w + TAB_GAP;
        }
        if !group.tabs.is_empty() {
            x -= TAB_GAP;
        }
    }
    if view.agents.is_empty() {
        let x = c.text(x + WORKSPACE_GAP, ty, Size::Ui, "no agents", DIM) + 18;
        let x = c.text(x, ty, Size::Ui, "super shift a", MID) + SPACE;
        c.text(x, ty, Size::Ui, "new", DIM);
    }
    // Clicking anywhere else on the bar opens the sheet
    hits.push((0, c.w, Hit::Sheet));
    Some(hits)
}

/// The line at the top of the sheet: what to type into, or how to start typing
const INPUT_H: i32 = 44;
/// Between that line's rule and the first heading, and the last row and the sheet's bottom
const SHEET_TOP: i32 = 8;
const SHEET_BOTTOM: i32 = 12;
/// Characters of name and machine the sheet shows, and the widest state word
const LIST_NAME_COLS: usize = 14;
const MACHINE_COLS: usize = 9;
const STATE_COLS: i32 = 7;
/// Behind the chosen row
const SELECTED: u32 = 0x1c1c1c;
/// Letters matched by what's typed
const MATCH: u32 = 0xffffff;

/// Where the sheet's icons go: a pixel in from its padding, so they're centred on a character cell there
const SHEET_ICON: i32 = SHEET_PAD - 1;

/// Where the sheet's columns start: name, machine, state, and what it says. The name is as far from its icon as in
/// the bar's tabs
fn grid() -> (i32, i32, i32, i32) {
    let advance = Size::Ui.font().advance;
    let name = SHEET_ICON + ICON + ICON_GAP;
    let machine = name + LIST_NAME_COLS as i32 * advance + 24;
    let state = machine + MACHINE_COLS as i32 * advance + 24;
    (name, machine, state, state + STATE_COLS * advance + 24)
}

/// One line of the sheet's list
enum Line {
    /// A section's title and how many rows it has
    Heading(&'static str, usize),
    /// The `shown` row with this index
    Row(usize),
    /// How many more there are below
    More(usize),
    /// Said when there are no rows
    Empty(&'static str),
}

/// Where each line of the list goes, and how tall the sheet is for them
fn layout(list: &Picker) -> (Vec<(i32, Line)>, i32) {
    let mut lines = Vec::new();
    let mut y = INPUT_H + 1 + SHEET_TOP;
    let mut group = None;
    let end = list.shown.len().min(list.top + SHEET_ROWS);
    for i in list.top..end {
        let (section, title) = list.heading(&list.rows[list.shown[i].0]);
        if group != Some(section) {
            if group.is_some() {
                y += GROUP_GAP;
            }
            group = Some(section);
            if let Some(title) = title {
                let count = list.shown.iter().filter(|(r, _)| list.heading(&list.rows[*r]).0 == section).count();
                lines.push((y, Line::Heading(title, count)));
                y += HEAD_H;
            }
        }
        lines.push((y, Line::Row(i)));
        y += ROW_H;
    }
    if list.shown.len() > end {
        lines.push((y, Line::More(list.shown.len() - end)));
        y += ROW_H;
    }
    if list.shown.is_empty() {
        let what = match list.mode {
            _ if list.looking => "looking…",
            _ => "nothing matches",
        };
        lines.push((y, Line::Empty(what)));
        y += ROW_H;
    }
    (lines, y + SHEET_BOTTOM)
}

/// The tallest the sheet gets, which its surface is sized for: a full list under all four headings, a gap after
/// them before the new agent row, and "+N more"
pub fn sheet_max_height() -> i32 {
    INPUT_H + 1 + SHEET_TOP + SHEET_ROWS as i32 * ROW_H + 4 * HEAD_H + 4 * GROUP_GAP + ROW_H + SHEET_BOTTOM
}

/// Draws the sheet at the top of the canvas: the line to type into, then the rows under their headings. Returns which spans of y do what when clicked, and how tall it is
pub fn sheet(c: &mut Canvas, view: &View, list: &Picker) -> (Vec<(i32, i32, Hit)>, i32) {
    c.fill(0, 0, c.w, c.h, 0);
    let (lines, h) = layout(list);
    debug_assert!(h <= c.h, "the sheet outgrew its surface");
    // Its edges shown by its surface against the windows alone
    c.raise = SHEET_RAISE;
    c.fill(0, 0, c.w, h, opaque(0));
    input(c, list);
    let mut hits = Vec::new();
    c.fill(0, INPUT_H, c.w, 1, opaque(RULE));

    let left = SHEET_PAD;
    let right = c.w - SHEET_PAD;
    let advance = Size::Ui.font().advance;
    let (name_x, machine_x, state_x, detail_x) = grid();
    // A question's answers and what went wrong are sentences, running on from the name
    let sentences = matches!(list.mode, Mode::End | Mode::Note);
    let typed = list.typed();

    for (y, line) in lines {
        let ty = text_top(Size::Ui, y + ROW_H / 2);
        let row = match line {
            Line::Heading(title, count) => {
                let hy = text_top(Size::Tiny, y + HEAD_H / 2);
                let tx = c.spaced(left, hy, Size::Tiny, title, DIM, 1);
                c.spaced(tx + 5, hy, Size::Tiny, &count.to_string(), COUNT, 1);
                continue;
            }
            Line::More(n) => {
                c.text(name_x, ty, Size::Ui, &format!("+{n} more"), DIM);
                continue;
            }
            Line::Empty(what) => {
                c.text(name_x, ty, Size::Ui, what, DIM);
                continue;
            }
            Line::Row(i) => i,
        };
        let (r, at) = &list.shown[row];
        let item = &list.rows[*r];
        // Searching, the pointer chooses the row it's over, so there's only ever one lit
        let selected = list.mode != Mode::Note && row == list.selected;
        // Where choosing follows the pointer, the chosen row already shows it
        let hovered = list.mode == Mode::End && view.pointer.is_some_and(|(_, py)| py >= y && py < y + ROW_H);
        if selected || hovered {
            c.fill(0, y, c.w, ROW_H, opaque(if selected { SELECTED } else { HOVER }));
        }
        // The window that's focused, marked at the left edge as the bar underlines it: quieter beside a chosen row
        if item.current {
            c.fill(0, y, 2, ROW_H, opaque(MID));
        }
        let mid = y + ROW_H / 2;
        if let Some((icon, colour)) = item.icon {
            c.icon(icon, SHEET_ICON, icon_top(mid), colour);
        }
        // The age, right-aligned, and what it says up to it
        let mut end = right;
        if item.since > 0 {
            let secs = view.now_ms.saturating_sub(item.since) / 1000;
            let age = age(secs);
            let bright = item.section == Section::Needs && secs >= STALE;
            c.text(right - width(Size::Ui, &age), ty, Size::Ui, &age, if bright { INK } else { DIM });
            end = right - 5 * advance;
        }
        let cols = |x: i32| ((end - SPACE - x) / advance).max(0) as usize;
        if sentences {
            let x = c.text(name_x, ty, Size::Ui, &cut(&item.name, cols(name_x)), item.name_colour);
            // Back on the grid when the sentence leaves room
            let dx = if x + SPACE <= state_x { state_x } else { x + 3 * SPACE };
            c.text(dx, ty, Size::Ui, &cut(&item.detail, cols(dx)), item.detail_colour);
        } else {
            // Where it is, in place of what it says, when that's what matched
            let place = item.detail == item.place || at.iter().any(|(field, _)| *field == PLACE);
            let (detail, detail_colour) = if place && item.detail != item.place { (&item.place, DIM) } else { (&item.detail, item.detail_colour) };
            // Once something's typed, the letters it matched are white and underlined and the rest of the name
            // steps back
            let name_colour = if typed { MID } else if selected { INK } else { item.name_colour };
            let columns = [
                (name_x, &item.name, LIST_NAME_COLS, name_colour, NAME),
                (machine_x, &item.machine, MACHINE_COLS, DIM, MACHINE),
                (state_x, &item.state, STATE_COLS as usize, item.state_colour, u8::MAX),
                (detail_x, detail, cols(detail_x), detail_colour, if place { PLACE } else { u8::MAX }),
            ];
            for (x, text, cols, colour, field) in columns {
                lit_text(c, x, ty, &cut(text, cols), colour, at, field);
            }
        }
        hits.push((y, y + ROW_H, Hit::Row(row, item.action.clone())));
    }
    c.raise = 0;
    (hits, h)
}

/// `text` with the characters `at` matched in `field` lit: white and underlined. Drawn in runs between them, and the
/// "…" of a cut never lit
fn lit_text(c: &mut Canvas, x: i32, y: i32, text: &str, colour: u32, at: &Match, field: u8) {
    let lit: Vec<usize> = at.iter().filter(|(f, _)| *f == field).map(|(_, i)| *i).collect();
    if lit.is_empty() {
        c.text(x, y, Size::Ui, text, colour);
        return;
    }
    let f = Size::Ui.font();
    let chars: Vec<char> = text.chars().collect();
    let mut x = x;
    let mut i = 0;
    while i < chars.len() {
        let on = |i: usize| lit.contains(&i) && chars[i] != '…';
        let run = (i..chars.len()).take_while(|&j| on(j) == on(i)).count();
        let s: String = chars[i..i + run].iter().collect();
        let next = c.text(x, y, Size::Ui, &s, if on(i) { MATCH } else { colour });
        if on(i) {
            c.fill(x, y + f.baseline + 2, next - x, 1, opaque(MATCH));
        }
        (x, i) = (next, i + run);
    }
}

/// The line at the top: what's typed, and what Enter does on the right; asking or saying what went wrong, the question
fn input(c: &mut Canvas, list: &Picker) {
    let right = c.w - SHEET_PAD;
    let (name_x, ..) = grid();
    let mid = INPUT_H / 2;
    let ty = text_top(Size::Ui, mid);
    // A key and what it does, the key brighter
    let hint = |c: &mut Canvas, x: i32, keys: &str, what: &str, colour: u32| {
        let kx = c.text(x, ty, Size::Ui, keys, MID);
        c.text(kx + SPACE, ty, Size::Ui, what, colour)
    };
    let hint_w = |keys: &str, what: &str| width(Size::Ui, keys) + SPACE + width(Size::Ui, what);
    match list.mode {
        Mode::Search | Mode::Projects => {
            c.icon(Icon::Search, SHEET_ICON, icon_top(mid), INK);
            let mut x = name_x;
            if !list.prompt.is_empty() {
                x = c.text(x, ty, Size::Ui, &list.prompt, MID) + SPACE;
            }
            // The end of a long query, which is where the caret is
            let query: Vec<char> = list.query.chars().map(printable).collect();
            let query: String = match query.len() {
                0..=48 => query.into_iter().collect(),
                n => std::iter::once('…').chain(query[n - 47..].iter().copied()).collect(),
            };
            x = c.text(x, ty, Size::Ui, &query, INK);
            // The caret a pixel into the cell where the next letter goes, the same before anything's typed as after,
            // so the first key moves it one cell like every other. The placeholder starts a cell past it
            let f = Size::Ui.font();
            c.fill(x + 1, ty + f.top - 1, 2, f.baseline - f.top + 3, opaque(INK));
            if list.query.is_empty() && list.mode == Mode::Search {
                c.text(x + f.advance, ty, Size::Ui, "agents, shells, projects", DIM);
            }
            if let Some(row) = list.selected_row() {
                let verb = row.action.verb();
                hint(c, right - hint_w("enter", verb), "enter", verb, DIM);
            }
        }
        Mode::End => {
            c.text(name_x, ty, Size::Ui, &list.prompt, INK);
        }
        Mode::Note => {
            c.icon(Icon::Error, SHEET_ICON, icon_top(mid), RED);
            c.text(name_x, ty, Size::Ui, &list.prompt, INK);
            hint(c, right - hint_w("esc", "close"), "esc", "close", DIM);
        }
    }
}

/// The fullscreen line: the screen's width shared out between `colours`, with a gap between each
pub fn line(c: &mut Canvas, colours: &[u32]) {
    c.fill(0, 0, c.w, c.h, 0);
    let n = colours.len() as i32;
    for (i, colour) in colours.iter().enumerate() {
        let (x0, x1) = (c.w * i as i32 / n.max(1), c.w * (i as i32 + 1) / n.max(1));
        c.fill(x0, 0, x1 - x0 - if i as i32 + 1 < n { 4 } else { 0 }, c.h, opaque(*colour));
    }
}
