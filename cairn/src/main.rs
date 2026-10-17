//! cairn: battery, wifi, CPU, memory and the time, written quietly onto the desktop in the sky left
//! of the wallpaper's mountain. No dependencies, Wayland is spoken directly over its socket, so it
//! works on any compositor with wlr-layer-shell (Hyprland, niri, sway).

mod font;
mod stats;
mod sys;
mod wayland;

use std::io;
use std::os::fd::AsRawFd;
use std::process::exit;
use std::time::{Duration, Instant};

use font::Size;
use stats::Stats;
use sys::{Map, PollFd, POLLIN};
use wayland::{Arg, Conn, Event, DISPLAY};

const REFRESH: Duration = Duration::from_secs(1);

/// Left edge of the text and its vertical middle, as fractions of the screen: the empty sky left of the mountain
const LEFT: f32 = 0.2;
const MIDDLE: f32 = 1.0 / 3.0;
/// Pixels between lines, on top of the font's own spacing
const LINE_GAP: i32 = 4;
/// Labels are padded to this many characters so the values line up
const LABEL_COLS: usize = 5;

/// Grey of dim labels and bright values, the wallpaper's white is about 230
const LABEL: u8 = 130;
const VALUE: u8 = 230;

/// Font size, and runs of text with their grey
type Line = (Size, Vec<(String, u8)>);

fn main() {
    let stats = Stats::new();
    let size = text_size(&layout(&stats.widest()));
    let mut desktop = Desktop::new(size, stats).unwrap_or_else(|err| fail(&format!("can't start: {err}")));

    loop {
        let timeout = (desktop.last_refresh + REFRESH).saturating_duration_since(Instant::now());
        let mut fds = [PollFd { fd: desktop.conn.fd(), events: POLLIN, revents: 0 }];
        if let Err(err) = sys::poll_fds(&mut fds, timeout.as_millis() as i32) {
            fail(&format!("poll failed: {err}"));
        }

        if fds[0].revents != 0 {
            for event in desktop.conn.read_events() {
                desktop.handle(event);
            }
        }
        let now = Instant::now();
        if now.duration_since(desktop.last_refresh) >= REFRESH {
            desktop.refresh(now);
        }
        if desktop.dirty {
            desktop.draw();
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("cairn: {msg}");
    exit(1);
}

struct Buffer {
    id: u32,
    busy: bool,
    /// The text it holds, so only lines that changed get drawn again
    shown: Vec<Line>,
}

/// Two buffers the size of the text in one shared pool, drawn into alternately
struct Pixels {
    map: Map,
    pool: u32,
    buffers: [Buffer; 2],
}

struct Desktop {
    conn: Conn,
    compositor: u32,
    shm: u32,
    layer_shell: u32,
    /// Empty input region, so clicks go through to the desktop underneath
    region: u32,
    /// (wl_surface, zwlr_layer_surface_v1)
    surface: (u32, u32),
    /// Width and height of the screen, from the first configure while the surface still covers all of it
    screen: Option<(i32, i32)>,
    /// Made once the surface has shrunk to the text
    pixels: Option<Pixels>,
    /// Width and height of the text, sized for the widest values so it doesn't jump around
    size: (i32, i32),
    last_refresh: Instant,
    stats: Stats,
    text: Vec<Line>,
    dirty: bool,
}

impl Desktop {
    fn new(size: (i32, i32), mut stats: Stats) -> io::Result<Desktop> {
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
        let layer_shell = bind(&mut conn, registry, &globals, "zwlr_layer_shell_v1", 1)?;
        let region = conn.new_id();
        conn.send(compositor, 1, &[Arg::Uint(region)]);

        // The first CPU reading is "-", it needs a second sample to compare against
        let text = layout(&stats.items());
        let mut desktop = Desktop {
            conn,
            compositor,
            shm,
            layer_shell,
            region,
            surface: (0, 0),
            screen: None,
            pixels: None,
            size,
            last_refresh: Instant::now(),
            stats,
            text,
            dirty: false,
        };
        desktop.create_surface();
        Ok(desktop)
    }

    fn handle(&mut self, event: Event) {
        let mut args = event.args();
        if event.obj == DISPLAY && event.op == 0 {
            let (obj, code, msg) = (args.uint(), args.uint(), args.string());
            fail(&format!("wayland error on object {obj} (code {code}): {msg}"));
        }
        let (surface, layer) = self.surface;
        if event.obj == layer {
            match event.op {
                // configure: ack it, then the next commit can carry a buffer
                0 => {
                    let (serial, width, height) = (args.uint(), args.uint() as i32, args.uint() as i32);
                    self.conn.send(layer, 6, &[Arg::Uint(serial)]);
                    if self.screen.is_none() {
                        self.screen = Some((width, height));
                        self.shrink(width, height);
                    } else if self.pixels.is_none() {
                        self.make_pixels();
                        self.dirty = true;
                    }
                }
                // closed by the compositor, e.g. the output went away, so start over on whichever is focused
                1 => {
                    self.conn.send(layer, 7, &[]);
                    self.conn.send(surface, 0, &[]);
                    self.drop_pixels();
                    self.screen = None;
                    self.create_surface();
                }
                _ => {}
            }
            return;
        }
        // wl_buffer.release
        for buffer in self.pixels.iter_mut().flat_map(|p| &mut p.buffers) {
            if event.obj == buffer.id && event.op == 0 {
                buffer.busy = false;
            }
        }
    }

    fn create_surface(&mut self) {
        let surface = self.conn.new_id();
        self.conn.send(self.compositor, 0, &[Arg::Uint(surface)]);
        let layer = self.conn.new_id();
        // Above the wallpaper, below every window
        const BOTTOM: u32 = 1;
        // Null output lets the compositor pick, which is the focused one
        self.conn.send(
            self.layer_shell,
            0,
            &[Arg::Uint(layer), Arg::Obj(surface), Arg::Obj(0), Arg::Uint(BOTTOM), Arg::Str("cairn")],
        );
        // Size 0 anchored to every edge is the whole screen, the first configure says how big that is
        self.conn.send(layer, 0, &[Arg::Uint(0), Arg::Uint(0)]);
        const ALL_EDGES: u32 = 15;
        self.conn.send(layer, 1, &[Arg::Uint(ALL_EDGES)]);
        // -1: measure from the screen's edges even if a bar reserves space
        self.conn.send(layer, 2, &[Arg::Int(-1)]);
        self.conn.send(surface, 5, &[Arg::Obj(self.region)]);
        self.conn.send(surface, 6, &[]);
        self.surface = (surface, layer);
    }

    /// Shrinks the surface from the whole screen to just the text, in its place in the sky
    fn shrink(&mut self, screen_w: i32, screen_h: i32) {
        let (surface, layer) = self.surface;
        let (w, h) = self.size;
        let left = (screen_w as f32 * LEFT) as i32;
        let top = ((screen_h as f32 * MIDDLE) as i32 - h / 2).max(0);
        self.conn.send(layer, 0, &[Arg::Uint(w as u32), Arg::Uint(h as u32)]);
        const TOP_LEFT: u32 = 1 | 4;
        self.conn.send(layer, 1, &[Arg::Uint(TOP_LEFT)]);
        self.conn.send(layer, 3, &[Arg::Int(top), Arg::Int(0), Arg::Int(0), Arg::Int(left)]);
        self.conn.send(surface, 6, &[]);
    }

    fn make_pixels(&mut self) {
        // A new memfd starts zeroed, which is transparent
        let (w, h) = self.size;
        let size = w * h * 4;
        let (file, map) = sys::memfd(c"cairn")
            .and_then(|f| f.set_len(2 * size as u64).map(|_| f))
            .and_then(|f| Map::new(&f, 2 * size as usize).map(|m| (f, m)))
            .unwrap_or_else(|err| fail(&format!("can't make the pixel buffer: {err}")));
        let pool = self.conn.new_id();
        self.conn.send_fd(self.shm, 0, &[Arg::Uint(pool), Arg::Int(2 * size)], file.as_raw_fd());
        let buffers = [0, 1].map(|i| {
            let id = self.conn.new_id();
            const ARGB8888: u32 = 0;
            self.conn.send(pool, 0, &[Arg::Uint(id), Arg::Int(i * size), Arg::Int(w), Arg::Int(h), Arg::Int(w * 4), Arg::Uint(ARGB8888)]);
            Buffer { id, busy: false, shown: Vec::new() }
        });
        self.pixels = Some(Pixels { map, pool, buffers });
    }

    fn drop_pixels(&mut self) {
        if let Some(pixels) = self.pixels.take() {
            for buffer in pixels.buffers {
                self.conn.send(buffer.id, 0, &[]);
            }
            self.conn.send(pixels.pool, 1, &[]);
        }
    }

    fn refresh(&mut self, now: Instant) {
        let text = layout(&self.stats.items());
        self.last_refresh = now;
        if text != self.text {
            self.text = text;
            self.dirty = true;
        }
    }

    fn draw(&mut self) {
        let Some(pixels) = &mut self.pixels else { return };
        // Still dirty if there's no free buffer, the release event wakes the loop to retry
        let Some(i) = pixels.buffers.iter().position(|b| !b.busy) else { return };
        self.dirty = false;

        // Only lines that differ from what this buffer holds are drawn again
        let (w, h) = self.size;
        let (surface, _) = self.surface;
        let px = &mut pixels.map.pixels()[(i as i32 * w * h) as usize..][..(w * h) as usize];
        let mut y = 0;
        for (n, line) in self.text.iter().enumerate() {
            let font = line.0.font();
            if pixels.buffers[i].shown.get(n) != Some(line) {
                px[(y * w) as usize..((y + font.height) * w) as usize].fill(0);
                let mut x = 0;
                for (s, grey) in &line.1 {
                    for c in s.chars() {
                        font.draw(px, w, x, y, c, *grey);
                        x += font.advance;
                    }
                }
                self.conn.send(surface, 2, &[Arg::Int(0), Arg::Int(y), Arg::Int(w), Arg::Int(font.height)]);
            }
            y += font.height + LINE_GAP;
        }

        self.conn.send(surface, 1, &[Arg::Obj(pixels.buffers[i].id), Arg::Int(0), Arg::Int(0)]);
        self.conn.send(surface, 6, &[]);
        pixels.buffers[i].busy = true;
        pixels.buffers[i].shown = self.text.clone();
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

/// The clock big on its own line, then a line per stat with its label
fn layout(items: &[(&str, String)]) -> Vec<Line> {
    items
        .iter()
        .map(|(label, value)| {
            if label.is_empty() {
                (Size::Clock, vec![(value.clone(), VALUE)])
            } else {
                (Size::Text, vec![(format!("{label:LABEL_COLS$}"), LABEL), (value.clone(), VALUE)])
            }
        })
        .collect()
}

/// Width and height in pixels
fn text_size(lines: &[Line]) -> (i32, i32) {
    let width = |(size, runs): &Line| runs.iter().map(|(s, _)| s.chars().count()).sum::<usize>() as i32 * size.font().advance;
    let w = lines.iter().map(width).max().unwrap_or(0);
    let h = lines.iter().map(|(size, _)| size.font().height + LINE_GAP).sum::<i32>() - LINE_GAP;
    (w, h)
}
