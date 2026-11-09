//! The bar across the top of the screen with every agent on it, the sheet that drops from it while SUPER is held
//! or after a click, and the line that stands in for the bar over a fullscreen window. See ui.rs for what they
//! show; this is the Wayland side. Clicking an agent's tab, or its row in the sheet, goes to its window

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::path::PathBuf;
use std::process::exit;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::agents::{Agent, Agents};
use crate::cmd;
use crate::draw::Canvas;
use crate::hc;
use crate::hypr::{self, Hypr, Place};
use crate::keys::{self, Keys};
use crate::picker::{self, Action, Mode, Picker, Shows};
use crate::stats::{Item, Stats};
use crate::sys::{self, Map, PollFd, POLLIN, SIGCMD};
use crate::ui::{self, Hit, View, BAR_H, SHEET_W};
use crate::wayland::{Arg, Conn, Event, DISPLAY};

/// How often the stats are read again. The bar is only drawn again if what it says changed
const REFRESH: Duration = Duration::from_secs(5);
/// How long a pill shows brighter after its agent starts wanting you
const FLASH: Duration = Duration::from_millis(900);
/// How long the projects found last are trusted before every machine is asked again
const PROJECTS_FRESH: Duration = Duration::from_secs(60);
/// How long the picker waits to be given the keyboard before closing, rather than stay open unable to hear Esc
const KEYBOARD_WAIT: Duration = Duration::from_secs(1);
/// The line over a fullscreen window
const LINE_H: i32 = 2;

// Wayland layers, opcodes and constants, see wlr-layer-shell-unstable-v1.xml, cursor-shape-v1.xml and wayland.xml
const TOP: u32 = 2;
const OVERLAY: u32 = 3;
const COMPOSITOR_CREATE_SURFACE: u16 = 0;
const COMPOSITOR_CREATE_REGION: u16 = 1;
const REGION_DESTROY: u16 = 0;
const REGION_ADD: u16 = 1;
const SURFACE_DESTROY: u16 = 0;
const SURFACE_ATTACH: u16 = 1;
const SURFACE_DAMAGE: u16 = 2;
const SURFACE_SET_INPUT_REGION: u16 = 5;
const SURFACE_COMMIT: u16 = 6;
const LAYER_SET_SIZE: u16 = 0;
const LAYER_SET_ANCHOR: u16 = 1;
const LAYER_SET_EXCLUSIVE_ZONE: u16 = 2;
const LAYER_SET_MARGIN: u16 = 3;
const LAYER_SET_KEYBOARD_INTERACTIVITY: u16 = 4;
const LAYER_ACK_CONFIGURE: u16 = 6;
const LAYER_DESTROY: u16 = 7;
const ANCHOR_TOP: u32 = 1;
const ANCHOR_LEFT: u32 = 4;
const ANCHOR_RIGHT: u32 = 8;
const SEAT_GET_POINTER: u16 = 0;
const SEAT_POINTER: u32 = 1;
const SEAT_KEYBOARD: u32 = 2;
const SEAT_GET_KEYBOARD: u16 = 1;
const CURSOR_GET_POINTER: u16 = 1;
const CURSOR_SET_SHAPE: u16 = 1;
const CURSOR_DEFAULT: u32 = 1;
const CURSOR_POINTER: u32 = 4;
const BTN_LEFT: u32 = 0x110;

pub fn run() -> ! {
    // First, so threads started later (the NVIDIA library starts one) inherit the block and can't take a signal
    // whose default is to end the process
    let mut signals = sys::signal_file(&[SIGCMD]).unwrap_or_else(|err| fail(&format!("can't take signals: {err}")));
    // One bar: starting another (after an install, say) replaces the one running, rather than stack two
    replace_running();
    // Then where to find this, so a command during the slow start (the NVIDIA library) waits rather than going nowhere
    write_pid();
    let mut desktop = Desktop::new().unwrap_or_else(|err| fail(&format!("can't start: {err}")));

    loop {
        let now = Instant::now();
        // Nothing's read or drawn while a fullscreen window hides the bar, unless the sheet is open over it
        let tick = (!desktop.fullscreen || desktop.sheet.show).then(|| (desktop.refreshed + REFRESH).min(next_minute(now)));
        let flash = desktop.flash.values().min().copied();
        let repeat = desktop.repeat.map(|(_, at)| at);
        let deaf = (desktop.picker.is_some() && !desktop.keyboard_entered).then(|| desktop.picker_opened + KEYBOARD_WAIT);
        let wake = [tick, flash, repeat, deaf, desktop.agents.retry_at(), desktop.hypr.retry_at()];
        let timeout = wake.into_iter().flatten().min().map_or(-1, |at| at.saturating_duration_since(now).as_millis() as i32 + 1);

        let mut fds = vec![
            PollFd { fd: desktop.conn.fd(), events: POLLIN, revents: 0 },
            PollFd { fd: signals.as_raw_fd(), events: POLLIN, revents: 0 },
            PollFd { fd: desktop.wake.0.as_raw_fd(), events: POLLIN, revents: 0 },
        ];
        let agents_fd = desktop.agents.fd();
        let hypr_fd = desktop.hypr.fd();
        fds.extend(agents_fd.map(|fd| PollFd { fd, events: POLLIN, revents: 0 }));
        fds.extend(hypr_fd.map(|fd| PollFd { fd, events: POLLIN, revents: 0 }));
        if let Err(err) = sys::poll_fds(&mut fds, timeout) {
            fail(&format!("poll failed: {err}"));
        }
        let ready = |fd: Option<i32>| fd.is_some_and(|fd| fds.iter().any(|p| p.fd == fd && p.revents != 0));

        if fds[0].revents != 0 {
            for event in desktop.conn.read_events() {
                desktop.handle(event);
            }
        }
        if fds[2].revents != 0 {
            desktop.jobs_done();
        }
        if fds[1].revents != 0 {
            for (_, what) in sys::read_signals(&mut signals) {
                desktop.command(what);
            }
        }
        if ready(agents_fd) && desktop.agents.read() {
            desktop.agents_changed();
        }
        if ready(hypr_fd) {
            let (mut moved, mut focused) = (false, false);
            for event in desktop.hypr.read() {
                match event {
                    hypr::Event::Focus(class) => {
                        focused = true;
                        if desktop.agents.focus(&class) {
                            desktop.agents_changed();
                        }
                        desktop.bar.dirty = true;
                        // Another window focused puts the picker away, unless it's the picker taking the keyboard
                        // as it opens
                        if desktop.picker_opened.elapsed() > Duration::from_millis(300) {
                            desktop.close_picker();
                        }
                    }
                    hypr::Event::Workspace(name) => {
                        if name != desktop.workspace {
                            (desktop.workspace, desktop.bar.dirty) = (name, true);
                        }
                    }
                    hypr::Event::Moved => moved = true,
                }
            }
            // Once for the lot, since a window opening alone sends several
            if moved {
                desktop.close_picker();
                desktop.update_fullscreen();
            }
            // On focus too, since flipping a split moves windows without an event of its own
            if moved || focused {
                desktop.update_places();
            }
        }

        let now = Instant::now();
        let before = desktop.flash.len();
        desktop.flash.retain(|_, until| *until > now);
        if desktop.flash.len() != before {
            desktop.bar.dirty = true;
        }
        if deaf.is_some_and(|at| at <= now) {
            desktop.close_picker();
        }
        if let Some((key, at)) = desktop.repeat.filter(|(_, at)| *at <= now) {
            let every = Duration::from_millis(1000 / desktop.repeat_rate.0.max(1) as u64);
            desktop.repeat = Some((key, at.max(now - every) + every));
            desktop.key(key);
        }
        desktop.agents.retry(now);
        desktop.hypr.retry(now);
        if tick.is_some_and(|at| at <= now) {
            desktop.refresh(now);
        }
        desktop.draw();
    }
}

/// When the clock next turns a minute, so it's never behind by up to a tick
fn next_minute(now: Instant) -> Instant {
    let into = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() % 60_000) as u64;
    now + Duration::from_millis(60_000 - into)
}

fn fail(msg: &str) -> ! {
    eprintln!("lookout: {msg}");
    exit(1);
}

/// Where `lookout pick` and the rest find the daemon
pub fn pid_file() -> PathBuf {
    PathBuf::from(env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("lookout.pid")
}

/// Ends the bar already running, if there is one, and waits a moment for it to go so its surfaces are gone first
fn replace_running() {
    let Some(pid) = cmd::daemon().filter(|&pid| pid != std::process::id() as i32) else { return };
    if sys::send_signal(pid, sys::SIGTERM, 0).is_err() {
        return;
    }
    let gone = Instant::now() + Duration::from_secs(2);
    while Instant::now() < gone && fs::metadata(format!("/proc/{pid}")).is_ok() {
        thread::sleep(Duration::from_millis(20));
    }
}

fn write_pid() {
    if let Err(err) = fs::write(pid_file(), std::process::id().to_string()) {
        eprintln!("lookout: can't write {}: {err}", pid_file().display());
    }
}

/// Two buffers the size of the surface in one shared pool, drawn into alternately
struct Pixels {
    map: Map,
    pool: u32,
    /// Each buffer's id, and whether the compositor still has it
    buffers: [(u32, bool); 2],
    size: (i32, i32),
}

#[derive(Clone, Copy, PartialEq)]
enum Which {
    Bar,
    Sheet,
    Line,
}

/// A layer surface, sized once
struct Surface {
    surface: u32,
    layer: u32,
    pixels: Option<Pixels>,
    /// Configured and ready for a buffer
    configured: bool,
    /// Showing a buffer
    mapped: bool,
    /// Whether it should be showing
    show: bool,
    dirty: bool,
}

impl Surface {
    fn new() -> Surface {
        Surface { surface: 0, layer: 0, pixels: None, configured: false, mapped: false, show: false, dirty: true }
    }
}

struct Desktop {
    conn: Conn,
    compositor: u32,
    shm: u32,
    layer_shell: u32,
    seat: u32,
    pointer: Option<u32>,
    /// Sets the pointer's shape over the bar, None if the compositor can't
    cursor_shape: Option<u32>,
    cursor_device: Option<u32>,
    bar: Surface,
    sheet: Surface,
    line: Surface,
    /// The surface the pointer's over, where on it, and the serial its arrival came with
    hover: Option<(u32, i32, i32)>,
    /// The surface and hit the pointer was last over, to redraw only when that changes
    hovered: Option<(u32, usize)>,
    enter_serial: u32,
    cursor: u32,
    /// What clicking where does on the bar (spans of x) and the sheet (spans of y), from their last drawing
    bar_hits: Vec<(i32, i32, Hit)>,
    sheet_hits: Vec<(i32, i32, Hit)>,
    /// A fullscreen window hides the bar, so the line shows instead
    fullscreen: bool,
    /// The name of the workspace showing
    workspace: String,
    stats: Stats,
    items: Vec<Item>,
    clock: (String, String),
    refreshed: Instant,
    agents: Agents,
    /// The agents in the bar's order, and where their windows are, which it comes from
    tabs: Vec<Agent>,
    places: HashMap<String, Place>,
    /// The agents in the bar's order
    list: Vec<Agent>,
    /// Agents that just started wanting you, until when their pill shows brighter
    flash: HashMap<String, Instant>,
    wanting: HashSet<String>,
    /// The first list from hc has arrived, so what wants you after it is news
    started: bool,
    sheet_height: i32,
    hypr: Hypr,
    keyboard: Option<u32>,
    keys: Keys,
    /// A key held down, and when it next repeats
    repeat: Option<(u32, Instant)>,
    /// Repeats a second, and the wait before they start, in ms
    repeat_rate: (i32, i32),
    /// The picker, while it's open, and when it opened
    picker: Option<Picker>,
    picker_opened: Instant,
    /// Slow work done off the loop sends what it found here, then wakes the loop with a byte down the pipe
    jobs: (Sender<Job>, Receiver<Job>),
    wake: (PipeReader, PipeWriter),
    /// What the last look at the terminals that report nothing found, and every project, for the picker to start
    /// from while it looks again
    shells: (HashMap<String, cmd::Detail>, HashMap<String, String>),
    projects: Option<Vec<(Option<String>, String)>>,
    /// When every machine last answered with its projects, and whether they're being asked now
    projects_found: Option<Instant>,
    projects_asking: bool,
    /// How many looks at the shells have been asked for, and which last answered
    shells_asked: u64,
    shells_answered: u64,
    /// The picker has the keyboard. If it hasn't soon after opening, it closes rather than sit there unable to hear
    keyboard_entered: bool,
}

/// What slow work done off the loop found
enum Job {
    Shells(u64, HashMap<String, cmd::Detail>, HashMap<String, String>),
    Projects(Vec<(Option<String>, String)>),
    Failed(String),
    /// Nothing to show for it
    Done,
}

impl Desktop {
    fn new() -> io::Result<Desktop> {
        let mut conn = Conn::connect()?;
        let registry = conn.new_id();
        conn.send(DISPLAY, 1, &[Arg::Uint(registry)]);
        let done = conn.new_id();
        conn.send(DISPLAY, 0, &[Arg::Uint(done)]);

        let mut globals = Vec::new();
        'roundtrip: loop {
            for event in conn.read_events() {
                if event.obj == registry && event.op == 0 {
                    let mut args = event.args();
                    globals.push((args.uint(), args.string(), args.uint()));
                } else if event.obj == done {
                    break 'roundtrip;
                }
            }
        }

        let compositor = bind(&mut conn, registry, &globals, "wl_compositor", 4)?;
        let shm = bind(&mut conn, registry, &globals, "wl_shm", 1)?;
        let layer_shell = bind(&mut conn, registry, &globals, "zwlr_layer_shell_v1", 2)?;
        let seat = bind(&mut conn, registry, &globals, "wl_seat", 5)?;
        let cursor_shape = bind(&mut conn, registry, &globals, "wp_cursor_shape_manager_v1", 1).ok();

        let stats = Stats::new();
        let (hypr, focused) = Hypr::new();
        let mut desktop = Desktop {
            conn,
            compositor,
            shm,
            layer_shell,
            seat,
            pointer: None,
            cursor_shape,
            cursor_device: None,
            bar: Surface::new(),
            sheet: Surface::new(),
            line: Surface::new(),
            hover: None,
            hovered: None,
            enter_serial: 0,
            cursor: 0,
            bar_hits: Vec::new(),
            sheet_hits: Vec::new(),
            fullscreen: hypr::fullscreen(),
            workspace: hypr::workspace(),
            clock: stats.clock(),
            stats,
            items: Vec::new(),
            refreshed: Instant::now(),
            agents: Agents::new(),
            tabs: Vec::new(),
            places: hypr::places().unwrap_or_default(),
            list: Vec::new(),
            flash: HashMap::new(),
            wanting: HashSet::new(),
            started: false,
            sheet_height: 0,
            hypr,
            keyboard: None,
            keys: Keys::new(),
            repeat: None,
            repeat_rate: (25, 600),
            picker: None,
            picker_opened: Instant::now(),
            jobs: mpsc::channel(),
            wake: io::pipe()?,
            shells: (HashMap::new(), HashMap::new()),
            projects: None,
            projects_found: None,
            projects_asking: false,
            shells_asked: 0,
            shells_answered: 0,
            keyboard_entered: false,
        };
        if let Some(class) = focused {
            desktop.agents.focus(&class);
        }
        // The first CPU reading is "-", it needs a second sample to compare against, so the next one is soon
        desktop.items = desktop.stats.items();
        desktop.refreshed = Instant::now().checked_sub(REFRESH - Duration::from_secs(1)).unwrap_or(desktop.refreshed);
        desktop.bar.show = true;
        for which in [Which::Bar, Which::Sheet, Which::Line] {
            desktop.create(which);
        }
        Ok(desktop)
    }

    fn surf(&mut self, which: Which) -> &mut Surface {
        match which {
            Which::Bar => &mut self.bar,
            Which::Sheet => &mut self.sheet,
            Which::Line => &mut self.line,
        }
    }

    fn which(&self, obj: u32) -> Option<Which> {
        [(self.bar.layer, Which::Bar), (self.sheet.layer, Which::Sheet), (self.line.layer, Which::Line)]
            .into_iter()
            .find(|(layer, _)| *layer == obj)
            .map(|(_, which)| which)
    }

    fn create(&mut self, which: Which) {
        let surface = self.conn.new_id();
        self.conn.send(self.compositor, COMPOSITOR_CREATE_SURFACE, &[Arg::Uint(surface)]);
        let layer = self.conn.new_id();
        let level = if which == Which::Bar { TOP } else { OVERLAY };
        // Null output lets the compositor pick, which is the focused one
        self.conn.send(self.layer_shell, 0, &[Arg::Uint(layer), Arg::Obj(surface), Arg::Obj(0), Arg::Uint(level), Arg::Str("lookout")]);
        let (anchor, size, zone, margin) = match which {
            // The bar takes its height from the top of the screen, so windows tile below it
            Which::Bar => (ANCHOR_TOP | ANCHOR_LEFT | ANCHOR_RIGHT, (0, BAR_H), BAR_H, 0),
            // The sheet floats under the bar, over the windows, as far from it and the edge as windows are
            // Anchored to the top only, so it's centred across the screen
            Which::Sheet => (ANCHOR_TOP, (SHEET_W, ui::sheet_max_height()), -1, ui::SHEET_Y),
            Which::Line => (ANCHOR_TOP | ANCHOR_LEFT | ANCHOR_RIGHT, (0, LINE_H), -1, 0),
        };
        self.conn.send(layer, LAYER_SET_SIZE, &[Arg::Uint(size.0 as u32), Arg::Uint(size.1 as u32)]);
        self.conn.send(layer, LAYER_SET_ANCHOR, &[Arg::Uint(anchor)]);
        self.conn.send(layer, LAYER_SET_EXCLUSIVE_ZONE, &[Arg::Int(zone)]);
        // Never the keyboard: it's the protocol's default, said outright so no surface of lookout's can come between
        // the keys and the compositor's binds
        self.conn.send(layer, LAYER_SET_KEYBOARD_INTERACTIVITY, &[Arg::Uint(0)]);
        self.conn.send(layer, LAYER_SET_MARGIN, &[Arg::Int(margin), Arg::Int(0), Arg::Int(0), Arg::Int(0)]);
        // The line is only to be seen, clicks go through it to the window
        if which == Which::Line {
            let region = self.region(0, 0);
            self.conn.send(surface, SURFACE_SET_INPUT_REGION, &[Arg::Obj(region)]);
            self.conn.send(region, REGION_DESTROY, &[]);
        }
        self.conn.send(surface, SURFACE_COMMIT, &[]);
        let surf = self.surf(which);
        (surf.surface, surf.layer, surf.configured, surf.mapped, surf.dirty) = (surface, layer, false, false, true);
        // A new surface has no input region yet
        if which == Which::Sheet {
            self.sheet_height = 0;
        }
    }

    /// A region of the top left `w` by `h`, to be destroyed once it's been used
    fn region(&mut self, w: i32, h: i32) -> u32 {
        let region = self.conn.new_id();
        self.conn.send(self.compositor, COMPOSITOR_CREATE_REGION, &[Arg::Uint(region)]);
        if w > 0 && h > 0 {
            self.conn.send(region, REGION_ADD, &[Arg::Int(0), Arg::Int(0), Arg::Int(w), Arg::Int(h)]);
        }
        region
    }

    fn destroy(&mut self, which: Which) {
        let (surface, layer, pixels) = {
            let surf = self.surf(which);
            (surf.surface, surf.layer, surf.pixels.take())
        };
        self.conn.send(layer, LAYER_DESTROY, &[]);
        self.conn.send(surface, SURFACE_DESTROY, &[]);
        if let Some(pixels) = pixels {
            for (id, _) in pixels.buffers {
                self.conn.send(id, 0, &[]);
            }
            self.conn.send(pixels.pool, 1, &[]);
        }
    }

    fn handle(&mut self, event: Event) {
        let mut args = event.args();
        if event.obj == DISPLAY && event.op == 0 {
            let (obj, code, msg) = (args.uint(), args.uint(), args.string());
            fail(&format!("wayland error on object {obj} (code {code}): {msg}"));
        }
        if let Some(which) = self.which(event.obj) {
            match event.op {
                // configure: ack it, then the next commit can carry a buffer
                0 => {
                    let (serial, w) = (args.uint(), args.uint() as i32);
                    let layer = self.surf(which).layer;
                    self.conn.send(layer, LAYER_ACK_CONFIGURE, &[Arg::Uint(serial)]);
                    let fixed = match which {
                        Which::Sheet => (SHEET_W, ui::sheet_max_height()),
                        Which::Bar => (w, BAR_H),
                        Which::Line => (w, LINE_H),
                    };
                    if self.surf(which).pixels.as_ref().is_none_or(|p| p.size != fixed) {
                        self.make_pixels(which, fixed);
                    }
                    let surf = self.surf(which);
                    (surf.configured, surf.dirty) = (true, true);
                }
                // closed by the compositor, e.g. the output went away, so start over on whichever is focused. Not
                // with the picker open, since the new surface wouldn't have the keyboard
                1 => {
                    self.close_picker();
                    self.destroy(which);
                    self.create(which);
                }
                _ => {}
            }
            return;
        }
        if event.obj == self.seat {
            // capabilities: a pointer to click the bar with, and a keyboard to type into the picker
            let caps = if event.op == 0 { args.uint() } else { 0 };
            if caps & SEAT_KEYBOARD != 0 && self.keyboard.is_none() {
                let keyboard = self.conn.new_id();
                self.conn.send(self.seat, SEAT_GET_KEYBOARD, &[Arg::Uint(keyboard)]);
                self.keyboard = Some(keyboard);
            }
            if caps & SEAT_POINTER != 0 && self.pointer.is_none() {
                let pointer = self.conn.new_id();
                self.conn.send(self.seat, SEAT_GET_POINTER, &[Arg::Uint(pointer)]);
                self.pointer = Some(pointer);
                if let Some(manager) = self.cursor_shape {
                    let device = self.conn.new_id();
                    self.conn.send(manager, CURSOR_GET_POINTER, &[Arg::Uint(device), Arg::Obj(pointer)]);
                    self.cursor_device = Some(device);
                }
            }
            return;
        }
        if Some(event.obj) == self.pointer {
            self.pointer_event(event.op, &mut args);
            return;
        }
        if Some(event.obj) == self.keyboard {
            self.keyboard_event(event.op, &mut args);
            return;
        }
        // wl_buffer.release
        if event.op == 0 {
            for pixels in [&mut self.bar.pixels, &mut self.sheet.pixels, &mut self.line.pixels].into_iter().flatten() {
                for (id, busy) in &mut pixels.buffers {
                    if *id == event.obj {
                        *busy = false;
                    }
                }
            }
        }
    }

    fn pointer_event(&mut self, op: u16, args: &mut crate::wayland::Args) {
        let fixed = |v: u32| v as i32 / 256;
        match op {
            // enter
            0 => {
                let (serial, surface, x, y) = (args.uint(), args.uint(), args.uint(), args.uint());
                (self.enter_serial, self.cursor) = (serial, 0);
                self.hover = Some((surface, fixed(x), fixed(y)));
            }
            // leave
            1 => self.hover = None,
            // motion
            2 => {
                let (_, x, y) = (args.uint(), args.uint(), args.uint());
                if let Some((surface, ..)) = self.hover {
                    self.hover = Some((surface, fixed(x), fixed(y)));
                }
            }
            // button
            3 => {
                let (_, _, button, state) = (args.uint(), args.uint(), args.uint(), args.uint());
                if button == BTN_LEFT && state == 1 {
                    self.click();
                }
                return;
            }
            _ => return,
        }
        // Drawn again only if the pointer moved onto something else
        let (hit, pointed) = match self.hit() {
            Some((n, hit)) => (Some((n, !matches!(hit, Hit::Sheet))), if let Hit::Row(i, _) = hit { Some(*i) } else { None }),
            None => (None, None),
        };
        let over = self.hover.map(|h| h.0).zip(hit.map(|h| h.0));
        if over != self.hovered {
            // Searching, moving onto a row chooses it, so Enter and a click agree. Only moving onto one, so a nudge
            // doesn't undo the arrow keys, and never in a question, where keeping is the choice until you move off it
            if let (Some(picker), Some(i)) = (self.picker.as_mut().filter(|p| p.mode != Mode::End), pointed) {
                picker.select(i);
            }
            for (surface, _) in [over, self.hovered].into_iter().flatten() {
                if surface == self.bar.surface {
                    self.bar.dirty = true;
                } else if surface == self.sheet.surface {
                    self.sheet.dirty = true;
                }
            }
            self.hovered = over;
        }
        // A hand over what can be clicked
        let shape = if self.hover.is_some_and(|h| h.0 == self.bar.surface) || hit.is_some_and(|h| h.1) { CURSOR_POINTER } else { CURSOR_DEFAULT };
        if let (Some(device), true) = (self.cursor_device, self.hover.is_some() && shape != self.cursor) {
            self.cursor = shape;
            self.conn.send(device, CURSOR_SET_SHAPE, &[Arg::Uint(self.enter_serial), Arg::Uint(shape)]);
        }
    }

    /// What's under the pointer, and its place in the hits
    fn hit(&self) -> Option<(usize, &Hit)> {
        let (surface, x, y) = self.hover?;
        let (hits, at) = if surface == self.bar.surface {
            (&self.bar_hits, x)
        } else if surface == self.sheet.surface && self.sheet.mapped {
            (&self.sheet_hits, y)
        } else {
            return None;
        };
        hits.iter().enumerate().find(|(_, (from, to, _))| at >= *from && at < *to).map(|(i, (.., hit))| (i, hit))
    }

    fn click(&mut self) {
        match self.hit().map(|(_, hit)| hit.clone()) {
            Some(Hit::Do(action) | Hit::Row(_, action)) => self.act(action),
            // The rest of the bar opens the search, or puts it away
            Some(Hit::Workspace(ws)) => hypr::go_to_workspace(ws),
            Some(Hit::Sheet) if self.picker.is_some() => self.close_picker(),
            Some(Hit::Sheet) => self.open_search(),
            None => {}
        }
    }

    fn make_pixels(&mut self, which: Which, size: (i32, i32)) {
        if let Some(old) = self.surf(which).pixels.take() {
            for (id, _) in old.buffers {
                self.conn.send(id, 0, &[]);
            }
            self.conn.send(old.pool, 1, &[]);
        }
        // A new memfd starts zeroed, which is transparent
        let (w, h) = size;
        let bytes = w * h * 4;
        let (file, map) = sys::memfd(c"lookout")
            .and_then(|f| f.set_len(2 * bytes as u64).map(|_| f))
            .and_then(|f| Map::new(&f, 2 * bytes as usize).map(|m| (f, m)))
            .unwrap_or_else(|err| fail(&format!("can't make the pixel buffer: {err}")));
        let pool = self.conn.new_id();
        self.conn.send_fd(self.shm, 0, &[Arg::Uint(pool), Arg::Int(2 * bytes)], file.as_raw_fd());
        let buffers = [0, 1].map(|i| {
            let id = self.conn.new_id();
            const ARGB8888: u32 = 0;
            self.conn.send(pool, 0, &[Arg::Uint(id), Arg::Int(i * bytes), Arg::Int(w), Arg::Int(h), Arg::Int(w * 4), Arg::Uint(ARGB8888)]);
            (id, false)
        });
        self.surf(which).pixels = Some(Pixels { map, pool, buffers, size });
    }

    /// The sheet shows while the picker's open
    fn update_sheet(&mut self) {
        let show = self.picker.is_some();
        if show != self.sheet.show {
            (self.sheet.show, self.sheet.dirty) = (show, true);
            if show {
                // Ages in it may be stale
                self.refresh(Instant::now());
            }
        }
    }

    /// What `lookout pick`, `new` and `kill` asked for
    fn command(&mut self, what: u64) {
        // Asked again for what's open puts it away
        let open = self.picker.as_ref().map(|p| match p.mode {
            Mode::Search => cmd::PICK,
            Mode::Projects => cmd::NEW,
            Mode::End => cmd::END,
            Mode::Note => 0,
        });
        if open == Some(what) {
            return self.close_picker();
        }
        match what {
            cmd::PICK => self.open_search(),
            cmd::NEW => self.open_projects(),
            cmd::END => self.open_end(),
            _ => {}
        }
    }

    /// Shows `picker` in the sheet, with the keyboard for as long as it's open
    fn open_picker(&mut self, picker: Picker) {
        // Without a keyboard there'd be no Esc, so no picker
        if self.keyboard.is_none() {
            return;
        }
        self.repeat = None;
        if self.picker.is_none() {
            self.keyboard_entered = false;
            self.conn.send(self.sheet.layer, LAYER_SET_KEYBOARD_INTERACTIVITY, &[Arg::Uint(1)]);
            self.picker_opened = Instant::now();
        }
        self.picker = Some(picker);
        self.update_sheet();
        self.sheet.dirty = true;
    }

    /// Puts the picker away and gives the keyboard back
    fn close_picker(&mut self) {
        if self.picker.take().is_some() {
            self.repeat = None;
            self.conn.send(self.sheet.layer, LAYER_SET_KEYBOARD_INTERACTIVITY, &[Arg::Uint(0)]);
            self.update_sheet();
            self.sheet.dirty = true;
        }
    }

    fn search_rows(&self) -> Vec<picker::Row> {
        let cwd = |addr: &str| self.agents.cwd(addr);
        let projects = self.projects.as_deref().unwrap_or_default();
        picker::search_rows(&self.list, &self.agents.others(), self.agents.focused(), &cwd, &self.shells.0, &self.shells.1, projects)
    }

    /// Asks every machine for its projects again, unless they answered a moment ago or are still answering
    fn find_projects(&mut self) {
        if !self.projects_asking && self.projects_found.is_none_or(|at| at.elapsed() > PROJECTS_FRESH) {
            self.projects_asking = true;
            self.spawn_job(|| Job::Projects(cmd::projects()));
        }
    }

    /// The sheet with a line to type into, straight away from what's known, then again once the shells and projects
    /// have been looked at
    fn open_search(&mut self) {
        let mut picker = Picker::new(Mode::Search, String::new(), self.search_rows());
        picker.looking = self.projects.is_none();
        self.open_picker(picker);
        self.find_projects();
        let others = self.agents.others();
        if !others.is_empty() {
            // Numbered, so an older look that answers late doesn't replace a newer one
            self.shells_asked += 1;
            let asked = self.shells_asked;
            self.spawn_job(move || {
                let (details, resumable) = cmd::shells(&others);
                Job::Shells(asked, details, resumable)
            });
        }
    }

    /// Every project, the ones found last time straight away, while every machine is asked again
    fn open_projects(&mut self) {
        let rows = picker::project_rows(self.projects.as_deref().unwrap_or_default(), Shows::Always);
        let mut picker = Picker::new(Mode::Projects, "new agent in".into(), rows);
        picker.looking = self.projects.is_none();
        self.open_picker(picker);
        self.find_projects();
    }

    /// Asks before ending the agent in the focused window
    fn open_end(&mut self) {
        let Some(addr) = self.agents.focused().map(String::from) else { return };
        let agent = self.list.iter().find(|a| a.addr == addr);
        let label = agent.map_or_else(|| addr.clone(), |a| a.label().to_string());
        let machine = agent.map_or(hc::HOST.as_str(), |a| a.device());
        self.open_picker(picker::end(&addr, &label, machine));
    }

    /// Runs `work` off the loop, which picks up what it found in jobs_done
    fn spawn_job(&self, work: impl FnOnce() -> Job + Send + 'static) {
        let sender = self.jobs.0.clone();
        let Ok(mut wake) = self.wake.1.try_clone() else { return };
        thread::spawn(move || {
            let _ = sender.send(work());
            let _ = wake.write_all(&[1]);
        });
    }

    fn jobs_done(&mut self) {
        let mut buf = [0u8; 64];
        let _ = self.wake.0.read(&mut buf);
        while let Ok(job) = self.jobs.1.try_recv() {
            match job {
                Job::Shells(asked, details, resumable) => {
                    if asked < self.shells_answered {
                        continue;
                    }
                    (self.shells, self.shells_answered) = ((details, resumable), asked);
                    self.update_search();
                }
                Job::Projects(projects) => {
                    (self.projects_found, self.projects_asking) = (Some(Instant::now()), false);
                    if let Some(picker) = self.picker.as_mut().filter(|p| p.mode == Mode::Projects) {
                        picker.looking = false;
                        picker.set_rows(picker::project_rows(&projects, Shows::Always));
                        self.sheet.dirty = true;
                    }
                    self.projects = Some(projects);
                    self.update_search();
                }
                Job::Failed(msg) => self.open_picker(picker::note(&msg)),
                Job::Done => {}
            }
        }
    }

    /// The search's rows again, after the agents or shells or projects changed under it
    fn update_search(&mut self) {
        if self.picker.as_ref().is_some_and(|p| p.mode == Mode::Search) {
            let rows = self.search_rows();
            if let Some(picker) = &mut self.picker {
                picker.looking = self.projects.is_none();
                picker.set_rows(rows);
            }
            self.sheet.dirty = true;
        }
    }

    /// Does what the picker's chosen row is for
    fn pick(&mut self) {
        if let Some(action) = self.picker.as_ref().and_then(|p| p.selected_row()).map(|r| r.action.clone()) {
            self.act(action);
        }
    }

    /// Does what a row or tab is for
    fn act(&mut self, action: Action) {
        match action {
            Action::Go { addr, label, resume } => {
                self.close_picker();
                thread::spawn(move || cmd::go(&addr, &label, resume.as_deref()));
            }
            Action::New => self.open_projects(),
            Action::Start { machine, dir } => {
                self.close_picker();
                let taken = self.agents.names();
                self.spawn_job(move || cmd::start(machine.as_deref(), &dir, &taken).map_or_else(Job::Failed, |_| Job::Done));
            }
            Action::End { addr, label } => {
                self.close_picker();
                self.spawn_job(move || cmd::end(&addr, &label).map_or_else(Job::Failed, |_| Job::Done));
            }
            Action::Close => self.close_picker(),
        }
    }

    fn keyboard_event(&mut self, op: u16, args: &mut crate::wayland::Args) {
        match op {
            // keymap: its format, then (with the fd that came along) its size
            0 => {
                let (format, size) = (args.uint(), args.uint());
                let Some(fd) = self.conn.take_fd() else { return };
                let file = unsafe { File::from_raw_fd(fd) };
                const XKB_V1: u32 = 1;
                // From the start whatever the offset: the compositor may have left it at the end writing it
                let mut text = vec![0; size as usize];
                if format == XKB_V1 && file.read_exact_at(&mut text, 0).is_ok() {
                    self.keys.set_keymap(&text);
                }
            }
            // enter: the picker has the keyboard
            1 => self.keyboard_entered = true,
            // leave: the picker lost the keyboard, so it goes rather than stay open unable to hear Esc
            2 => {
                self.repeat = None;
                self.close_picker();
            }
            // key
            3 => {
                let (_, _, key, state) = (args.uint(), args.uint(), args.uint(), args.uint());
                if state == 1 {
                    self.key(key);
                    // Never the keys that pick or close: held, they'd act on whatever opens next
                    let acts = matches!(key, keys::ENTER | keys::KP_ENTER | keys::ESC);
                    if self.picker.is_some() && !acts && self.repeat_rate.0 > 0 && self.keys.repeats(key) {
                        self.repeat = Some((key, Instant::now() + Duration::from_millis(self.repeat_rate.1.max(0) as u64)));
                    }
                } else if self.repeat.is_some_and(|(held, _)| held == key) {
                    self.repeat = None;
                }
            }
            // modifiers
            4 => {
                let (_, depressed, latched, locked, group) = (args.uint(), args.uint(), args.uint(), args.uint(), args.uint());
                self.keys.modifiers(depressed, latched, locked, group);
            }
            // repeat_info
            5 => self.repeat_rate = (args.uint() as i32, args.uint() as i32),
            _ => {}
        }
    }

    /// A key pressed in the picker
    fn key(&mut self, key: u32) {
        if self.picker.is_none() {
            return;
        }
        let ctrl = self.keys.held(c"Control");
        match key {
            // SUPER as well as Esc, since SUPER opened it. It reaches here as the search has the keyboard
            keys::ESC | keys::SUPER_L | keys::SUPER_R => return self.close_picker(),
            keys::ENTER | keys::KP_ENTER => return self.pick(),
            _ => {}
        }
        // What a key types, unless it's a shortcut
        let text = if ctrl || self.keys.held(c"Mod4") { String::new() } else { self.keys.text(key) };
        let Some(picker) = &mut self.picker else { return };
        // Ctrl with p k, n j, u and w, found by where they are on the keyboard
        const P: u32 = 25;
        const K: u32 = 37;
        const N: u32 = 49;
        const J: u32 = 36;
        const U: u32 = 22;
        const W: u32 = 17;
        match key {
            keys::UP => picker.move_by(-1),
            keys::DOWN | keys::TAB => picker.move_by(1),
            keys::PAGE_UP => picker.move_by(-(ui::SHEET_ROWS as i32)),
            keys::PAGE_DOWN => picker.move_by(ui::SHEET_ROWS as i32),
            keys::BACKSPACE => picker.backspace(ctrl),
            P | K if ctrl => picker.move_by(-1),
            N | J if ctrl => picker.move_by(1),
            U if ctrl => picker.clear(),
            W if ctrl => picker.backspace(true),
            _ if !text.is_empty() => picker.type_text(&text),
            _ => return,
        }
        self.sheet.dirty = true;
    }

    fn update_fullscreen(&mut self) {
        let was = self.fullscreen;
        self.fullscreen = hypr::fullscreen();
        if was && !self.fullscreen {
            // It's been a while, so straight away rather than at the next tick
            self.refresh(Instant::now());
        }
        self.update_line();
    }

    fn update_line(&mut self) {
        let show = self.fullscreen && !ui::line_colours(&self.list).is_empty();
        if show != self.line.show || show {
            (self.line.show, self.line.dirty) = (show, true);
        }
    }

    fn refresh(&mut self, now: Instant) {
        self.refreshed = now;
        let (items, clock) = (self.stats.items(), self.stats.clock());
        if items != self.items || clock != self.clock {
            (self.items, self.clock) = (items, clock);
            self.bar.dirty = true;
        }
        self.sheet.dirty |= self.sheet.show;
    }

    fn agents_changed(&mut self) {
        let mut list = self.agents.list();
        ui::order(&mut list);
        let wanting: HashSet<String> = list.iter().filter(|a| a.urgency().is_some()).map(|a| a.addr.clone()).collect();
        if self.started {
            let until = Instant::now() + FLASH;
            for addr in wanting.difference(&self.wanting) {
                self.flash.insert(addr.clone(), until);
            }
        }
        self.started = true;
        // For `lookout next`, most wanting first
        let next: String = list.iter().filter(|a| wanting.contains(&a.addr)).map(|a| format!("{}\t{}\n", a.addr, a.label())).collect();
        let _ = fs::write(cmd::seen_file(), next);
        (self.list, self.wanting) = (list, wanting);
        self.arrange_tabs();
        self.sheet.dirty |= self.sheet.show;
        self.update_line();
        self.update_search();
    }

    /// Looks where the agents' windows are now, rearranging the tabs if that's changed
    fn update_places(&mut self) {
        let Some(places) = hypr::places().filter(|p| *p != self.places) else { return };
        self.places = places;
        self.arrange_tabs();
    }

    fn arrange_tabs(&mut self) {
        self.tabs = self.list.clone();
        ui::by_place(&mut self.tabs, &self.places);
        self.bar.dirty = true;
    }

    fn draw(&mut self) {
        for which in [Which::Bar, Which::Sheet, Which::Line] {
            self.draw_one(which);
        }
    }

    fn draw_one(&mut self, which: Which) {
        let surf = self.surf(which);
        if !surf.dirty {
            return;
        }
        if !surf.show {
            surf.dirty = false;
            if surf.mapped {
                // A null buffer unmaps it. Mapping again starts from a commit without one, then a configure
                let surface = surf.surface;
                (surf.mapped, surf.configured) = (false, false);
                self.conn.send(surface, SURFACE_ATTACH, &[Arg::Obj(0), Arg::Int(0), Arg::Int(0)]);
                self.conn.send(surface, SURFACE_COMMIT, &[]);
                self.conn.send(surface, SURFACE_COMMIT, &[]);
            }
            return;
        }
        // Still dirty if it isn't ready or there's no free buffer: the configure or release wakes the loop to retry
        if !surf.configured {
            return;
        }
        let Some(mut pixels) = surf.pixels.take() else { return };
        let Some(i) = pixels.buffers.iter().position(|(_, busy)| !busy) else {
            self.surf(which).pixels = Some(pixels);
            return;
        };
        let (w, h) = pixels.size;
        let px = &mut pixels.map.pixels()[(i as i32 * w * h) as usize..][..(w * h) as usize];
        let mut canvas = Canvas::new(px, w, h);
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let pointer = |hover: Option<(u32, i32, i32)>, surface: u32| hover.filter(|h| h.0 == surface).map(|(_, x, y)| (x, y));
        let flash = |addr: &str| self.flash.contains_key(addr);
        let view = View {
            agents: &self.tabs,
            focused: self.agents.focused(),
            stats: &self.items,
            clock: (&self.clock.0, &self.clock.1),
            places: &self.places,
            workspace: &self.workspace,
            now_ms,
            flash: &flash,
            pointer: None,
        };
        let surface = match which {
            Which::Bar => self.bar.surface,
            Which::Sheet => self.sheet.surface,
            Which::Line => self.line.surface,
        };
        let mut damage_h = h;
        match which {
            Which::Bar => self.bar_hits = ui::bar(&mut canvas, &View { pointer: pointer(self.hover, surface), ..view }),
            // It only shows while the picker's open, so there's always one to draw. Were there not, it's left clear
            Which::Sheet => match self.picker.as_ref() {
                Some(list) => {
                    let (hits, height) = ui::sheet(&mut canvas, &View { pointer: pointer(self.hover, surface), ..view }, list);
                    self.sheet_hits = hits;
                    // Only the panel changes, or the panel it was if that was taller, rather than the whole surface
                    damage_h = height.max(self.sheet_height).max(1);
                    // Clicks only land on the panel, not the clear space below it
                    if height != self.sheet_height {
                        self.sheet_height = height;
                        let region = self.region(SHEET_W, height);
                        self.conn.send(surface, SURFACE_SET_INPUT_REGION, &[Arg::Obj(region)]);
                        self.conn.send(region, REGION_DESTROY, &[]);
                    }
                }
                None => canvas.fill(0, 0, w, h, 0),
            },
            Which::Line => ui::line(&mut canvas, &ui::line_colours(&self.list)),
        }
        let id = pixels.buffers[i].0;
        pixels.buffers[i].1 = true;
        self.conn.send(surface, SURFACE_DAMAGE, &[Arg::Int(0), Arg::Int(0), Arg::Int(w), Arg::Int(damage_h)]);
        self.conn.send(surface, SURFACE_ATTACH, &[Arg::Obj(id), Arg::Int(0), Arg::Int(0)]);
        self.conn.send(surface, SURFACE_COMMIT, &[]);
        let surf = self.surf(which);
        (surf.pixels, surf.mapped, surf.dirty) = (Some(pixels), true, false);
    }
}

fn bind(conn: &mut Conn, registry: u32, globals: &[(u32, String, u32)], interface: &str, max_version: u32) -> io::Result<u32> {
    let (name, _, version) = globals
        .iter()
        .find(|g| g.1 == interface)
        .ok_or_else(|| io::Error::other(format!("compositor doesn't support {interface}")))?;
    let id = conn.new_id();
    conn.send(registry, 0, &[Arg::Uint(*name), Arg::Str(interface), Arg::Uint((*version).min(max_version)), Arg::Uint(id)]);
    Ok(id)
}
