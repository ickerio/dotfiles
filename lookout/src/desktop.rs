//! The board in the sky left of the wallpaper's mountain: the clock, the stats, then every agent. Hold SUPER to peek
//! at it over the windows. When an agent wants you and you're not peeking, one line says so in the top right corner

use std::env;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::exit;
use std::time::{Duration, Instant};

use crate::agents::{Agent, Agents};
use crate::cmd;
use crate::font::{Size, CHECKER, MARK_ADVANCE, OUTLINE, SOLID, SPARSE};
use crate::hypr::{self, Hypr};
use crate::stats::{Item, Stats};
use crate::sys::{self, Map, PollFd, POLLIN, SIGUSR1, SIGUSR2};
use crate::wayland::{Arg, Conn, Event, DISPLAY};

const REFRESH: Duration = Duration::from_secs(1);
/// How long SUPER is held before the board peeks. Hyprland's own long press waits a whole key repeat delay
const PEEK_DELAY: Duration = Duration::from_millis(200);

/// Left edge of the stats and their vertical middle, as fractions of the screen: the empty sky left of the mountain.
/// Agents hang below
const LEFT: f32 = 0.2;
const MIDDLE: f32 = 1.0 / 3.0;
/// Pixels between lines, on top of the font's own spacing
const LINE_GAP: i32 = 4;
/// Labels are padded to this many characters so the values line up. Agents use the same columns: their state is
/// the label, their name the value
const LABEL_COLS: usize = 5;
/// Agent names are cut to this, and what one says in the alert to NOTE_COLS
const NAME_COLS: usize = 12;
const NOTE_COLS: usize = 36;
/// Agents listed when peeking before the rest become "+N more". Peeking is for detail, so it's generous
const MAX_AGENTS: usize = 12;
/// Dithered pixel markers before each agent, like the wallpaper's mountain
const MARKERS: bool = true;
/// On the wallpaper, agents are one line of markers and counts. Their names are for when you peek
const SUMMARY: bool = true;
/// Room around the board's text, where its panel shows while peeking
const PAD: i32 = 28;
/// The panel's edge from the nearest ink on every side, measured from what's drawn rather than the glyphs' cells
const INSET: i32 = 24;
/// The alert's gap from the screen's corner, the same as Hyprland's outer gap so it lines up with windows
const ALERT_MARGIN: i32 = 6;
/// Extra space between the stats and the agents, so the gap matches the one under the clock
const SECTION_GAP: i32 = 13;

/// Colours as 0xRRGGBB. The wallpaper's white is about 230 grey. Brightness is how much something wants you, and
/// amber, the only colour, is an agent asking you something
const LABEL: u32 = 0x828282;
const VALUE: u32 = 0xe6e6e6;
/// The quietest grey: which machine an agent is on, and agents that ask nothing (idle, or done and seen)
const DEVICE: u32 = 0x5a5a5a;
const ASK: u32 = 0xf0b44c;
const ERROR: u32 = 0xf05545;
/// Premultiplied ARGB behind the text when it's over windows: the wallpaper's black, solid so nothing shows through
const PANEL: u32 = 0xff000000;

/// Runs of text, each with its font size and colour, sharing a baseline
type Line = Vec<(Size, String, u32)>;

// Wayland layers and opcodes, see wlr-layer-shell-unstable-v1.xml
const BOTTOM: u32 = 1;
const OVERLAY: u32 = 3;
const SURFACE_ATTACH: u16 = 1;
const SURFACE_DAMAGE: u16 = 2;
const SURFACE_SET_INPUT_REGION: u16 = 5;
const SURFACE_COMMIT: u16 = 6;
const LAYER_SET_SIZE: u16 = 0;
const LAYER_SET_ANCHOR: u16 = 1;
const LAYER_SET_EXCLUSIVE_ZONE: u16 = 2;
const LAYER_SET_MARGIN: u16 = 3;
const LAYER_ACK_CONFIGURE: u16 = 6;
const LAYER_DESTROY: u16 = 7;
const LAYER_SET_LAYER: u16 = 8;
const TOP: u32 = 1;
const LEFT_EDGE: u32 = 4;
const RIGHT_EDGE: u32 = 8;
const ALL_EDGES: u32 = 15;

pub fn run() -> ! {
    // First, so threads started later (the NVIDIA library starts one) inherit the block and can't take a signal
    // whose default is to end the process
    let mut signals = sys::signal_file(&[SIGUSR1, SIGUSR2]).unwrap_or_else(|err| fail(&format!("can't take signals: {err}")));
    // Then where to find this, so a peek during the slow start (the NVIDIA library) waits rather than going nowhere
    write_pid();
    let mut desktop = Desktop::new().unwrap_or_else(|err| fail(&format!("can't start: {err}")));

    loop {
        let now = Instant::now();
        // A board under windows isn't refreshed, so it doesn't wake anything up
        let tick = desktop.board_visible().then_some(desktop.last_refresh + REFRESH);
        let wake = [tick, desktop.peek_at, desktop.agents.retry_at(), desktop.hypr.retry_at()];
        let wake = wake.into_iter().flatten().min();
        let timeout = wake.map_or(-1, |wake| wake.saturating_duration_since(now).as_millis() as i32);

        let mut fds = vec![
            PollFd { fd: desktop.conn.fd(), events: POLLIN, revents: 0 },
            PollFd { fd: signals.as_raw_fd(), events: POLLIN, revents: 0 },
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
        if fds[1].revents != 0 {
            for signal in sys::read_signals(&mut signals) {
                // SUPER went down: peek if it's still held, and nothing has moved, once the delay is up
                if signal == SIGUSR1 && !desktop.peeking {
                    desktop.peek_at.get_or_insert(Instant::now() + PEEK_DELAY);
                } else if signal == SIGUSR2 {
                    desktop.peek(false);
                }
            }
        }
        if ready(agents_fd) && desktop.agents.read() {
            desktop.agents_changed();
        }
        if ready(hypr_fd) {
            for event in desktop.hypr.read() {
                match event {
                    hypr::Event::Focus(class) => {
                        if desktop.agents.focus(&class) {
                            desktop.agents_changed();
                        }
                        desktop.peek(false);
                    }
                    hypr::Event::Moved => {
                        desktop.peek(false);
                        desktop.update_cover();
                    }
                }
            }
        }

        let now = Instant::now();
        if desktop.peek_at.is_some_and(|at| at <= now) {
            desktop.peek(true);
        }
        desktop.agents.retry(now);
        desktop.hypr.retry(now);
        if desktop.board_visible() {
            if now.duration_since(desktop.last_refresh) >= REFRESH {
                desktop.refresh(now);
            }
        }
        // Drawn once even if covered, since a surface that's never been mapped can't be moved over the windows
        if desktop.board_visible() || !desktop.board.mapped {
            desktop.board.draw(&mut desktop.conn);
        }
        desktop.alert.draw(&mut desktop.conn);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("lookout: {msg}");
    exit(1);
}

/// Where `lookout peek` finds the daemon
pub fn pid_file() -> PathBuf {
    PathBuf::from(env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into())).join("lookout.pid")
}

fn write_pid() {
    if let Err(err) = fs::write(pid_file(), std::process::id().to_string()) {
        eprintln!("lookout: can't write {}: {err}", pid_file().display());
    }
}

struct Buffer {
    id: u32,
    busy: bool,
    /// The lines and background it holds, so only what changed gets drawn again
    shown: Vec<Line>,
    shown_bg: u32,
}

/// Two buffers the size of the surface in one shared pool, drawn into alternately
struct Pixels {
    map: Map,
    pool: u32,
    buffers: [Buffer; 2],
}

/// A layer surface showing lines of text
struct Panel {
    surface: u32,
    layer: u32,
    /// Width and height in pixels, padding included
    size: (i32, i32),
    pixels: Option<Pixels>,
    text: Vec<Line>,
    /// Premultiplied ARGB behind the text, 0 for none
    bg: u32,
    /// Hug the right edge, with the background only behind the text
    right: bool,
    /// Where the text starts: its left edge (from the right edge when hugging it) and its top
    origin: (i32, i32),
    /// Lines starting with a marker hang it into this much margin left of the text, so their text lines up with
    /// the lines without one
    hang: i32,
    dirty: bool,
    /// Configured and ready for a buffer
    configured: bool,
    /// Showing a buffer. Only the alert unmaps, when it has nothing to say, so it costs the compositor nothing
    mapped: bool,
}

impl Panel {
    fn draw(&mut self, conn: &mut Conn) {
        if !self.dirty || !self.configured {
            return;
        }
        if self.right && self.text.is_empty() {
            self.dirty = false;
            if self.mapped {
                // A null buffer unmaps it. Mapping again starts from a commit without one, then a configure
                conn.send(self.surface, SURFACE_ATTACH, &[Arg::Obj(0), Arg::Int(0), Arg::Int(0)]);
                conn.send(self.surface, SURFACE_COMMIT, &[]);
                conn.send(self.surface, SURFACE_COMMIT, &[]);
                self.mapped = false;
                self.configured = false;
            }
            return;
        }
        let Some(pixels) = &mut self.pixels else { return };
        // Still dirty if there's no free buffer, the release event wakes the loop to retry
        let Some(i) = pixels.buffers.iter().position(|b| !b.busy) else { return };
        self.dirty = false;

        let (w, h) = self.size;
        let px = &mut pixels.map.pixels()[(i as i32 * w * h) as usize..][..(w * h) as usize];
        let buffer = &mut pixels.buffers[i];
        // A background hugs the text, so it's drawn whole whenever there is one
        let whole = buffer.shown_bg != self.bg || self.bg != 0;
        // Hugging the right edge, the ink ends INSET from it
        let x0 = if self.right { w - INSET - ink_box(&self.text, 0, 0, self.hang).2 } else { self.origin.0 + self.hang };
        let top_y = self.origin.1;
        if whole {
            px.fill(0);
            if self.bg != 0 && !self.text.is_empty() {
                let (l, t, r, b) = ink_box(&self.text, x0, top_y, self.hang);
                let (l, t, r, b) = ((l - INSET).max(0), (t - INSET).max(0), (r + INSET).min(w), (b + INSET).min(h));
                for row in px[(t * w) as usize..(b * w) as usize].chunks_exact_mut(w as usize) {
                    row[l as usize..r as usize].fill(self.bg);
                }
            }
            conn.send(self.surface, SURFACE_DAMAGE, &[Arg::Int(0), Arg::Int(0), Arg::Int(w), Arg::Int(h)]);
        }

        let mut y = top_y;
        for (n, line) in self.text.iter().enumerate() {
            let (height, baseline) = line_metrics(line);
            if whole || buffer.shown.get(n) != Some(line) {
                // A whole redraw has already laid the background
                if !whole {
                    for row in px[(y * w) as usize..((y + height) * w) as usize].chunks_exact_mut(w as usize) {
                        row[PAD as usize..(w - PAD) as usize].fill(self.bg);
                    }
                }
                let mut x = line_start(line, x0, self.hang);
                for (size, s, colour) in line {
                    let font = size.font();
                    for c in s.chars() {
                        font.draw(px, w, x, y + baseline - font.baseline, c, *colour);
                        x += font.advance;
                    }
                }
                conn.send(self.surface, SURFACE_DAMAGE, &[Arg::Int(0), Arg::Int(y), Arg::Int(w), Arg::Int(height)]);
            }
            y += height + LINE_GAP;
        }
        // Lines that went away
        if !whole && buffer.shown.len() > self.text.len() {
            let bottom = (h - PAD).max(y);
            for row in px[(y * w) as usize..(bottom * w) as usize].chunks_exact_mut(w as usize) {
                row[PAD as usize..(w - PAD) as usize].fill(self.bg);
            }
            conn.send(self.surface, SURFACE_DAMAGE, &[Arg::Int(0), Arg::Int(y), Arg::Int(w), Arg::Int(bottom - y)]);
        }

        conn.send(self.surface, SURFACE_ATTACH, &[Arg::Obj(buffer.id), Arg::Int(0), Arg::Int(0)]);
        conn.send(self.surface, SURFACE_COMMIT, &[]);
        self.mapped = true;
        buffer.busy = true;
        buffer.shown = self.text.clone();
        buffer.shown_bg = self.bg;
    }

    fn set_text(&mut self, text: Vec<Line>) {
        if text != self.text {
            self.text = text;
            self.dirty = true;
        }
    }

    fn make_pixels(&mut self, conn: &mut Conn, shm: u32) {
        // A new memfd starts zeroed, which is transparent
        let (w, h) = self.size;
        let size = w * h * 4;
        let (file, map) = sys::memfd(c"lookout")
            .and_then(|f| f.set_len(2 * size as u64).map(|_| f))
            .and_then(|f| Map::new(&f, 2 * size as usize).map(|m| (f, m)))
            .unwrap_or_else(|err| fail(&format!("can't make the pixel buffer: {err}")));
        let pool = conn.new_id();
        conn.send_fd(shm, 0, &[Arg::Uint(pool), Arg::Int(2 * size)], file.as_raw_fd());
        let buffers = [0, 1].map(|i| {
            let id = conn.new_id();
            const ARGB8888: u32 = 0;
            conn.send(pool, 0, &[Arg::Uint(id), Arg::Int(i * size), Arg::Int(w), Arg::Int(h), Arg::Int(w * 4), Arg::Uint(ARGB8888)]);
            Buffer { id, busy: false, shown: Vec::new(), shown_bg: 0 }
        });
        self.pixels = Some(Pixels { map, pool, buffers });
        self.dirty = true;
    }

    fn drop_pixels(&mut self, conn: &mut Conn) {
        if let Some(pixels) = self.pixels.take() {
            for buffer in pixels.buffers {
                conn.send(buffer.id, 0, &[]);
            }
            conn.send(pixels.pool, 1, &[]);
        }
    }

    fn release(&mut self, obj: u32) {
        for buffer in self.pixels.iter_mut().flat_map(|p| &mut p.buffers) {
            if buffer.id == obj {
                buffer.busy = false;
            }
        }
    }
}

struct Desktop {
    conn: Conn,
    compositor: u32,
    shm: u32,
    layer_shell: u32,
    /// Empty input region, so clicks go through to whatever's underneath
    region: u32,
    board: Panel,
    alert: Panel,
    /// Width and height of the screen, from the board's first configure while it still covers all of it
    screen: Option<(i32, i32)>,
    /// Height of the stats alone, which stay centred where they always were
    stats_height: i32,
    peeking: bool,
    /// When a peek that's waiting out PEEK_DELAY shows, unless SUPER is let go or something moves first
    peek_at: Option<Instant>,
    /// Windows cover the board, so there's no point refreshing it
    covered: bool,
    last_refresh: Instant,
    stats: Stats,
    agents: Agents,
    hypr: Hypr,
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
        // Version 2 can move a surface between layers, which is how peeking works
        let layer_shell = bind(&mut conn, registry, &globals, "zwlr_layer_shell_v1", 2)?;
        let region = conn.new_id();
        conn.send(compositor, 1, &[Arg::Uint(region)]);

        // Sized once for the widest values and the most agents, so nothing jumps around
        let stats = Stats::new();
        let mut widest = stats_layout(&stats.widest());
        let stats_height = text_size(&widest).1;
        widest.push(spacer());
        widest.extend((0..MAX_AGENTS).map(|_| agent_line(&widest_agent(), NAME_COLS, 8)));
        let (board_w, board_h) = text_size(&widest);
        // The alert is sized and placed by its ink, so its panel is even on every side
        let alert = [alert_line(&widest_agent(), 99)];
        let (l, t, r, b) = ink_box(&alert, 0, 0, 0);
        let alert_origin = (INSET - l, INSET - t);
        let alert_size = (r - l + 2 * INSET, b - t + 2 * INSET);

        let panel = |size, right, origin, hang| Panel {
            surface: 0,
            layer: 0,
            size,
            pixels: None,
            text: Vec::new(),
            bg: 0,
            right,
            origin,
            hang,
            dirty: false,
            configured: false,
            mapped: false,
        };
        let focused = hypr::focused_class();
        let mut desktop = Desktop {
            conn,
            compositor,
            shm,
            layer_shell,
            region,
            board: panel((hang() + board_w + 2 * PAD, board_h + 2 * PAD), false, (PAD, PAD), hang()),
            alert: panel(alert_size, true, alert_origin, 0),
            screen: None,
            stats_height,
            peeking: false,
            peek_at: None,
            covered: hypr::covered(),
            last_refresh: Instant::now(),
            stats,
            agents: Agents::new(),
            hypr: Hypr::new(),
        };
        if let Some(class) = focused {
            desktop.agents.focus(&class);
        }
        // The first CPU reading is "-", it needs a second sample to compare against
        let text = desktop.board_text();
        desktop.board.set_text(text);
        desktop.create_board();
        desktop.create_alert();
        Ok(desktop)
    }

    fn handle(&mut self, event: Event) {
        let mut args = event.args();
        if event.obj == DISPLAY && event.op == 0 {
            let (obj, code, msg) = (args.uint(), args.uint(), args.string());
            fail(&format!("wayland error on object {obj} (code {code}): {msg}"));
        }
        if event.obj == self.board.layer {
            match event.op {
                // configure: ack it, then the next commit can carry a buffer
                0 => {
                    let (serial, width, height) = (args.uint(), args.uint() as i32, args.uint() as i32);
                    self.conn.send(self.board.layer, LAYER_ACK_CONFIGURE, &[Arg::Uint(serial)]);
                    if self.screen.is_none() {
                        self.screen = Some((width, height));
                        self.shrink_board(width, height);
                    } else if self.board.pixels.is_none() {
                        self.board.make_pixels(&mut self.conn, self.shm);
                        self.board.configured = true;
                    }
                }
                // closed by the compositor, e.g. the output went away, so start over on whichever is focused
                1 => {
                    self.destroy(true);
                    self.screen = None;
                    self.create_board();
                }
                _ => {}
            }
            return;
        }
        if event.obj == self.alert.layer {
            match event.op {
                0 => {
                    let serial = args.uint();
                    self.conn.send(self.alert.layer, LAYER_ACK_CONFIGURE, &[Arg::Uint(serial)]);
                    if self.alert.pixels.is_none() {
                        self.alert.make_pixels(&mut self.conn, self.shm);
                    }
                    self.alert.configured = true;
                    self.alert.dirty = true;
                }
                1 => {
                    self.destroy(false);
                    self.create_alert();
                }
                _ => {}
            }
            return;
        }
        // wl_buffer.release
        if event.op == 0 {
            self.board.release(event.obj);
            self.alert.release(event.obj);
        }
    }

    fn layer_surface(&mut self, layer: u32) -> (u32, u32) {
        let surface = self.conn.new_id();
        self.conn.send(self.compositor, 0, &[Arg::Uint(surface)]);
        let layer_surface = self.conn.new_id();
        // Null output lets the compositor pick, which is the focused one
        self.conn.send(
            self.layer_shell,
            0,
            &[Arg::Uint(layer_surface), Arg::Obj(surface), Arg::Obj(0), Arg::Uint(layer), Arg::Str("lookout")],
        );
        // -1: measure from the screen's edges even if a bar reserves space
        self.conn.send(layer_surface, LAYER_SET_EXCLUSIVE_ZONE, &[Arg::Int(-1)]);
        self.conn.send(surface, SURFACE_SET_INPUT_REGION, &[Arg::Obj(self.region)]);
        (surface, layer_surface)
    }

    fn create_board(&mut self) {
        let layer = if self.peeking { OVERLAY } else { BOTTOM };
        let (surface, layer) = self.layer_surface(layer);
        // Size 0 anchored to every edge is the whole screen, the first configure says how big that is
        self.conn.send(layer, LAYER_SET_SIZE, &[Arg::Uint(0), Arg::Uint(0)]);
        self.conn.send(layer, LAYER_SET_ANCHOR, &[Arg::Uint(ALL_EDGES)]);
        self.conn.send(surface, SURFACE_COMMIT, &[]);
        (self.board.surface, self.board.layer) = (surface, layer);
    }

    /// Shrinks the board from the whole screen to its own size, with the stats in their place in the sky
    fn shrink_board(&mut self, screen_w: i32, screen_h: i32) {
        let (w, h) = self.board.size;
        let left = (screen_w as f32 * LEFT) as i32 - PAD - self.board.hang;
        let top = ((screen_h as f32 * MIDDLE) as i32 - self.stats_height / 2 - PAD).max(0);
        let layer = self.board.layer;
        self.conn.send(layer, LAYER_SET_SIZE, &[Arg::Uint(w as u32), Arg::Uint(h as u32)]);
        self.conn.send(layer, LAYER_SET_ANCHOR, &[Arg::Uint(TOP | LEFT_EDGE)]);
        self.conn.send(layer, LAYER_SET_MARGIN, &[Arg::Int(top), Arg::Int(0), Arg::Int(0), Arg::Int(left.max(0))]);
        self.conn.send(self.board.surface, SURFACE_COMMIT, &[]);
    }

    fn create_alert(&mut self) {
        let (surface, layer) = self.layer_surface(OVERLAY);
        let (w, h) = self.alert.size;
        self.conn.send(layer, LAYER_SET_SIZE, &[Arg::Uint(w as u32), Arg::Uint(h as u32)]);
        self.conn.send(layer, LAYER_SET_ANCHOR, &[Arg::Uint(TOP | RIGHT_EDGE)]);
        let m = ALERT_MARGIN;
        self.conn.send(layer, LAYER_SET_MARGIN, &[Arg::Int(m), Arg::Int(m), Arg::Int(0), Arg::Int(0)]);
        self.conn.send(surface, SURFACE_COMMIT, &[]);
        (self.alert.surface, self.alert.layer) = (surface, layer);
        self.alert.bg = PANEL;
    }

    fn destroy(&mut self, board: bool) {
        let panel = if board { &mut self.board } else { &mut self.alert };
        self.conn.send(panel.layer, LAYER_DESTROY, &[]);
        self.conn.send(panel.surface, 0, &[]);
        panel.drop_pixels(&mut self.conn);
        (panel.configured, panel.mapped) = (false, false);
    }

    /// Raises the board over the windows with a panel behind it, or puts it back on the wallpaper
    fn peek(&mut self, on: bool) {
        self.peek_at = None;
        if on == self.peeking {
            return;
        }
        self.peeking = on;
        self.conn.send(self.board.layer, LAYER_SET_LAYER, &[Arg::Uint(if on { OVERLAY } else { BOTTOM })]);
        // Applied now, since a covered board isn't drawn (and committed) until it can be seen again
        self.conn.send(self.board.surface, SURFACE_COMMIT, &[]);
        self.board.bg = if on { PANEL } else { 0 };
        self.board.dirty = true;
        // The alert says less than the board, so it steps aside
        self.agents_changed();
    }

    fn board_visible(&self) -> bool {
        self.peeking || !self.covered
    }

    /// Asks Hyprland again whether windows cover the board, after something moved
    fn update_cover(&mut self) {
        let was_visible = self.board_visible();
        self.covered = hypr::covered();
        if self.board_visible() && !was_visible {
            // It's been a while, so straight away rather than at the next tick
            self.refresh(Instant::now());
        }
    }

    fn refresh(&mut self, now: Instant) {
        self.last_refresh = now;
        let text = self.board_text();
        self.board.set_text(text);
    }

    fn agents_changed(&mut self) {
        let text = self.board_text();
        self.board.set_text(text);
        let agents = self.agents.list();
        let wanting: Vec<&Agent> = agents.iter().filter(|a| a.urgency().is_some()).collect();
        // For `lookout next`
        let list: String = wanting.iter().map(|a| format!("{}\t{}\n", a.addr, a.label())).collect();
        let _ = fs::write(cmd::seen_file(), list);
        let alert = match wanting.first() {
            Some(first) if !self.peeking => vec![alert_line(first, wanting.len() - 1)],
            _ => Vec::new(),
        };
        self.alert.set_text(alert);
    }

    fn board_text(&mut self) -> Vec<Line> {
        let mut text = stats_layout(&self.stats.items());
        let agents = self.agents.list();
        if SUMMARY && !self.peeking {
            let summary = summary_line(&agents);
            if !summary.is_empty() {
                text.extend([spacer(), summary]);
            }
            return text;
        }
        if !agents.is_empty() {
            text.push(spacer());
        }
        let shown = if agents.len() > MAX_AGENTS { MAX_AGENTS - 1 } else { agents.len() };
        // Columns as wide as the widest of what's shown, so machines and requests line up down the rows
        let names = agents[..shown].iter().map(|a| cut(a.label(), NAME_COLS).len()).max().unwrap_or(0);
        let devices = agents[..shown].iter().map(|a| a.device().len()).max().unwrap_or(0);
        text.extend(agents[..shown].iter().map(|a| agent_line(a, names, devices)));
        if shown < agents.len() {
            text.push(vec![(Size::Text, " ".repeat(LABEL_COLS), LABEL), (Size::Small, format!("+{} more", agents.len() - shown), DEVICE)]);
        }
        text
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

/// The clock big on its own line, then a line per stat with its label, and any note small after a space
fn stats_layout(items: &[Item]) -> Vec<Line> {
    items
        .iter()
        .map(|(label, value, note)| {
            if label.is_empty() {
                return vec![(Size::Clock, value.clone(), VALUE)];
            }
            let mut line = vec![(Size::Text, format!("{label:LABEL_COLS$}"), LABEL), (Size::Text, value.clone(), VALUE)];
            if let Some(note) = note {
                // "3.6/15" reads as used of total, "38 chg" as a remark
                if !note.starts_with('/') {
                    line.push((Size::Text, " ".into(), VALUE));
                }
                line.push((Size::Small, note.clone(), LABEL));
            }
            line
        })
        .collect()
}

/// How an agent shows: its state as a label in the stats' style and the label's colour, and its name's colour. The
/// brighter, the more it wants you
struct Look {
    mark: char,
    label: &'static str,
    label_colour: u32,
    name_colour: u32,
}

fn look(agent: &Agent) -> Look {
    let (mark, label, label_colour, name_colour) = match agent.state.as_str() {
        "blocked" => (SOLID, "ASK", ASK, VALUE),
        "error" => (SOLID, "ERR", ERROR, VALUE),
        "done" if !agent.seen => (SOLID, "DONE", VALUE, VALUE),
        "done" => (OUTLINE, "DONE", DEVICE, DEVICE),
        "working" => (CHECKER, "WORK", LABEL, LABEL),
        // Busy too, on its background tasks, which wake it when they end
        "waiting" => (CHECKER, "WAIT", LABEL, LABEL),
        "idle" => (SPARSE, "IDLE", DEVICE, DEVICE),
        _ => (SPARSE, "?", DEVICE, DEVICE),
    };
    Look { mark, label, label_colour, name_colour }
}

/// An empty line between the stats and the agents
fn spacer() -> Line {
    vec![(Size::Small, String::new(), LABEL)]
}

/// "ASK  api     laptop", laid out like "MEM  3.6/15", with the name and machine padded to `names` and `devices` so
/// the columns line up. What an agent asks goes in the alert, where there's room to read it
fn agent_line(agent: &Agent, names: usize, devices: usize) -> Line {
    let look = look(agent);
    let mut line = marker(&look);
    line.extend([
        (Size::Text, format!("{:LABEL_COLS$}", look.label), look.label_colour),
        (Size::Text, format!("{:names$}  ", cut(agent.label(), NAME_COLS)), look.name_colour),
        (Size::Small, format!("{:devices$}", agent.device()), DEVICE),
    ]);
    line
}

/// "■ 2   ■ 1   ▩ 3": how many agents look each way, most wanting first, each count in its state's colour. Only
/// agents that want you or are working: idle ones, and finished ones you've seen, ask nothing of a glance
fn summary_line(agents: &[Agent]) -> Line {
    let mut groups: Vec<(char, u32, usize)> = Vec::new();
    for look in agents.iter().map(look).filter(|l| l.mark != SPARSE && l.mark != OUTLINE) {
        match groups.iter_mut().find(|(mark, colour, _)| *mark == look.mark && *colour == look.label_colour) {
            Some(group) => group.2 += 1,
            None => groups.push((look.mark, look.label_colour, 1)),
        }
    }
    let mut line = Line::new();
    for (i, (mark, colour, count)) in groups.into_iter().enumerate() {
        if i > 0 {
            line.push((Size::Text, "   ".into(), LABEL));
        }
        line.push((Size::Mark, mark.to_string(), colour));
        line.push((Size::Text, count.to_string(), colour));
    }
    line
}

/// What an agent that wants you says about it
fn note(agent: &Agent) -> Option<&str> {
    (agent.urgency().is_some() || agent.state == "blocked").then(|| agent.msg.as_deref().or(agent.kind.as_deref())).flatten()
}

/// "ASK  api  laptop  npm publish  +2", the first of what wants you, where, what it says, and how many others do
fn alert_line(agent: &Agent, others: usize) -> Line {
    let look = look(agent);
    let mut line = marker(&look);
    line.extend([
        (Size::Text, format!("{:LABEL_COLS$}", look.label), look.label_colour),
        (Size::Text, cut(agent.label(), NAME_COLS), look.name_colour),
        // Two text widths apart, like the label from the name
        (Size::Text, "  ".into(), LABEL),
        (Size::Small, agent.device(), DEVICE),
    ]);
    if let Some(note) = note(agent) {
        line.push((Size::Text, "  ".into(), LABEL));
        line.push((Size::Small, cut(note, NOTE_COLS), LABEL));
    }
    // How many others want you, not "more" like the board's hidden rows
    if others > 0 {
        line.push((Size::Text, "  ".into(), LABEL));
        line.push((Size::Small, format!("+{others}"), DEVICE));
    }
    line
}

/// The biggest an agent's line gets, to size the surfaces once: as wide as allowed, with the characters that reach
/// highest and lowest, so whatever's shown fits with its panel even around it
fn widest_agent() -> Agent {
    let tall = |n: usize| "Wd|(gj)".chars().cycle().take(n).collect::<String>();
    Agent {
        addr: format!("{}/t1", tall(8)),
        name: Some(tall(NAME_COLS)),
        state: "blocked".into(),
        kind: None,
        msg: Some(tall(NOTE_COLS)),
        since: 0,
        seen: false,
    }
}

/// `s` as printable ASCII, cut to `cols` characters with ".." when longer
fn cut(s: &str, cols: usize) -> String {
    let ascii: String = s.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '?' }).collect();
    if ascii.chars().count() > cols {
        let cut: String = ascii.chars().take(cols - 2).collect();
        // End at the last whole word rather than a scrap of one, unless it's all one word. The next character tells
        // whether the cut fell between words
        let between = ascii.chars().nth(cols - 2) == Some(' ');
        let cut = match cut.rsplit_once(' ') {
            Some((before, _)) if !between && !before.trim().is_empty() => before.to_string(),
            _ => cut,
        };
        format!("{}..", cut.trim_end())
    } else {
        ascii
    }
}

/// Height of a line and how far down its baseline sits, so smaller runs sit on the same baseline
fn line_metrics(line: &Line) -> (i32, i32) {
    if *line == spacer() {
        return (SECTION_GAP, 0);
    }
    let baseline = line.iter().map(|(size, ..)| size.font().baseline).max().unwrap_or(0);
    let height = line.iter().map(|(size, ..)| baseline - size.font().baseline + size.font().height).max().unwrap_or(0);
    (height, baseline)
}

/// Width and height in pixels
fn text_size(lines: &[Line]) -> (i32, i32) {
    let width = |line: &Line| line.iter().map(|(size, s, _)| s.chars().count() as i32 * size.font().advance).sum::<i32>();
    let w = lines.iter().map(width).max().unwrap_or(0);
    let h = lines.iter().map(|line| line_metrics(line).0 + LINE_GAP).sum::<i32>() - LINE_GAP;
    (w, h.max(0))
}


/// How far to move a line so its first ink lines up with the labels' rather than its cell. Only the clock's big
/// digits have enough space before them to matter
fn nudge(line: &Line) -> i32 {
    match line.first() {
        Some((Size::Clock, ..)) => {
            let left = |size: Size, c| size.font().ink(c).map_or(0, |ink| ink[0]);
            left(Size::Text, 'B') - left(Size::Clock, '0')
        }
        _ => 0,
    }
}

/// Where a line's first cell goes, given the text's left edge: markers hang left of it, and the clock moves so
/// its digits line up with the labels
fn line_start(line: &Line, x0: i32, hang: i32) -> i32 {
    x0 + nudge(line) - if hang > 0 && hangs(line) { MARK_ADVANCE } else { 0 }
}

/// Left, top, right and bottom (exclusive) of the ink `lines` lay down from (x, y), walking them as they're drawn
fn ink_box(lines: &[Line], x: i32, y: i32, hang: i32) -> (i32, i32, i32, i32) {
    let mut ink = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    let mut top = y;
    for line in lines {
        let (height, baseline) = line_metrics(line);
        let mut left = line_start(line, x, hang);
        for (size, s, _) in line {
            let font = size.font();
            let cell_top = top + baseline - font.baseline;
            for c in s.chars() {
                if let Some([l, t, r, b]) = font.ink(c) {
                    ink = (ink.0.min(left + l), ink.1.min(cell_top + t), ink.2.max(left + r), ink.3.max(cell_top + b));
                }
                left += font.advance;
            }
        }
        top += height + LINE_GAP;
    }
    if ink.0 > ink.2 { (x, y, x, y) } else { ink }
}

/// Whether a line starts with a marker
fn hangs(line: &Line) -> bool {
    line.first().is_some_and(|(size, ..)| *size == Size::Mark)
}

/// The marker run that starts an agent's line, if markers are on
fn marker(look: &Look) -> Line {
    if MARKERS { vec![(Size::Mark, look.mark.to_string(), look.label_colour)] } else { Vec::new() }
}

/// The margin markers hang into
fn hang() -> i32 {
    if MARKERS { MARK_ADVANCE } else { 0 }
}
