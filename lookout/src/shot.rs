//! `lookout shot <out.png> [options]`: draws the bar and sheet with made-up agents into a PNG, to see a design
//! change without restarting the daemon.
//!
//!   --bg <file.ppm>   what's behind, a binary PPM as big as the screen (e.g. the wallpaper)
//!   --size WxH        the screen, 1920x1080 by default
//!   --windows         two tiled terminals under the bar
//!   --window          one terminal alone under the bar, flush with it as Hyprland lays out a lone window
//!   --fullscreen      a fullscreen window, so only the line shows
//!   --many | --few | --none | --calm    how many agents, and whether any want you
//!   --pointer X,Y     where the pointer is, to see hovering
//!   --pick | --new | --end | --note    the sheet searching, as SUPER + A, SUPER + SHIFT + A or SUPER + SHIFT + Q
//!                     opens it, or saying what went wrong
//!   --type TEXT       typed into the search
//!   --flash           the first pill as it shows when it has just started wanting you

use std::fs;
use std::process::exit;
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::HashMap;

use crate::agents::Agent;
use crate::hc::Term;
use crate::picker::{self, Mode, Picker, Shows};
use crate::draw::{opaque, Canvas};
use crate::ui::{self, View, BAR_H, SHEET_W};

pub fn run(args: &[String]) {
    let Some(out) = args.first() else {
        eprintln!("usage: lookout shot <out.png> [--bg file.ppm] [--size WxH] [--windows] [--pick|--new|--end|--note] [--type TEXT] [--fullscreen] [--many|--few|--none|--calm] [--pointer X,Y] [--flash]");
        exit(2);
    };
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (w, h) = value("--size")
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1920, 1080));
    let pointer = value("--pointer").and_then(|s| s.split_once(',').and_then(|(x, y)| Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))));

    let mut px = vec![opaque(0); (w * h) as usize];
    if let Some(bg) = value("--bg") {
        load_ppm(&bg, &mut px, w, h);
    }
    let mut screen = Canvas::new(&mut px, w, h);

    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let mut agents = demo(now_ms, if flag("--many") { "many" } else if flag("--few") { "few" } else if flag("--none") { "none" } else if flag("--calm") { "calm" } else { "" });
    ui::order(&mut agents);
    let (items, clock) = (
        vec![("BAT", "84".to_string(), None), ("WIFI", "91".into(), None), ("CPU", "12".into(), None), ("MEM", "3.6".into(), Some("/15".into()))],
        ("14:32", "Sat 7 Nov"),
    );
    let first = agents.first().map(|a| a.addr.clone()).filter(|_| flag("--flash"));
    let flash = |addr: &str| first.as_deref() == Some(addr);
    // The sheet as SUPER + A, SUPER + SHIFT + A or SUPER + SHIFT + Q opens it, with --type's letters typed
    let projects: Vec<(Option<String>, String)> = ["/home/harry", "/home/harry/dotfiles", "/home/harry/ronin", "/home/harry/herdr-core"]
        .iter()
        .flat_map(|d| [(Some("desktop".to_string()), d.to_string()), (None, d.to_string())])
        .collect();
    let cwd = |addr: &str| if addr.contains("t5") || addr.contains("t7") { "/home/harry/ronin".to_string() } else { "/home/harry/dotfiles".to_string() };
    let mut list = if flag("--pick") {
        let others = vec![
            Term { addr: "t2".into(), name: Some("notes-old".into()), status: None, cwd: "/home/harry/notes".into() },
            Term { addr: "desktop/t3".into(), name: None, status: None, cwd: "/home/harry".into() },
        ];
        let resumable = HashMap::from([("t2".to_string(), "abc".to_string())]);
        let rows = picker::search_rows(&agents, &others, Some("t10"), &cwd, &HashMap::new(), &resumable, &projects);
        Some(Picker::new(Mode::Search, String::new(), rows))
    } else if flag("--new") {
        Some(Picker::new(Mode::Projects, "new agent in".into(), picker::project_rows(&projects, Shows::Always)))
    } else if flag("--end") {
        Some(picker::end("desktop/t5", "ronin", "desktop"))
    } else if flag("--note") {
        Some(picker::note("can't start dotfiles: hc: no such machine desktop"))
    } else {
        None
    };
    if let (Some(list), Some(typed)) = (&mut list, value("--type")) {
        list.type_text(&typed);
    }
    let view = |pointer| View {
        agents: &agents,
        focused: Some("t10"),
        stats: &items,
        clock,
        workspace: "2",
        now_ms,
        flash: &flash,
        pointer,
    };

    if flag("--fullscreen") {
        screen.fill(0, 0, w, h, opaque(0x141414));
        terminal(&mut screen, 0, 0, w, h, "~/ronin  cargo build --release");
        ui::line(&mut sub(&mut screen, 0, 0, w, 2), &ui::line_colours(&agents));
        return save(out, &px, w, h);
    }
    if flag("--window") {
        terminal(&mut screen, 0, BAR_H, w, h - BAR_H, "~/dotfiles  claude");
    }
    if flag("--windows") {
        let half = (w - 18) / 2;
        terminal(&mut screen, 6, BAR_H + 6, half, h - BAR_H - 12, "~/dotfiles  claude");
        terminal(&mut screen, 12 + half, BAR_H + 6, half, h - BAR_H - 12, "~/ronin  cargo test");
    }
    let bar_pointer = pointer.filter(|p| p.1 < BAR_H);
    ui::bar(&mut sub(&mut screen, 0, 0, w, BAR_H), &view(bar_pointer));
    if let Some(list) = &list {
        let sh = ui::sheet_max_height();
        let mut sheet = vec![0u32; (SHEET_W * sh) as usize];
        // Centred, as the compositor places it
        let (sx, sy) = ((w - SHEET_W) / 2, ui::SHEET_Y);
        let sheet_pointer = pointer.filter(|p| p.1 >= sy).map(|(x, y)| (x - sx, y - sy));
        ui::sheet(&mut Canvas::new(&mut sheet, SHEET_W, sh), &view(sheet_pointer), list);
        over(&mut screen, &sheet, sx, sy, SHEET_W, sh);
    }
    save(out, &px, w, h);
}

/// Made-up agents covering every state
fn demo(now: u64, which: &str) -> Vec<Agent> {
    let agent = |addr: &str, name: &str, state: &str, msg: Option<&str>, ago: u64| Agent {
        addr: addr.into(),
        name: Some(name.into()),
        state: state.into(),
        kind: None,
        msg: msg.map(String::from),
        since: now - ago * 1000,
        seen: false,
        tasks: if state == "waiting" { 2 } else { 0 },
    };
    let seen = |mut a: Agent| {
        a.seen = true;
        a
    };
    let mut agents = vec![
        agent("laptop/t3", "api", "blocked", Some("allow npm publish to the registry?"), 250),
        agent("desktop/t9", "web", "error", Some("tsc exited with code 2"), 760),
        agent("desktop/t5", "ronin", "done", Some("finished the parser refactor"), 70),
        agent("t10", "dotfiles", "working", Some("Reading /home/harry/dotfiles/lookout/src/desktop.rs"), 40),
        agent("desktop/t7", "ronin-2", "waiting", None, 190),
        agent("t4", "notes", "idle", None, 3900),
        seen(agent("t2", "scratch", "done", Some("answered"), 1300)),
    ];
    match which {
        "none" => agents.clear(),
        "few" => agents.retain(|a| a.name.as_deref() == Some("dotfiles") || a.name.as_deref() == Some("api")),
        "calm" => agents.retain(|a| a.urgency().is_none()),
        "many" => {
            for (i, (name, state)) in [
                ("infra", "working"),
                ("blog", "idle"),
                ("cli", "blocked"),
                ("ml-eval", "working"),
                ("docs", "done"),
                ("site", "idle"),
                ("bench", "waiting"),
                ("parser", "working"),
            ]
            .iter()
            .enumerate()
            {
                agents.push(agent(&format!("desktop/t{}", 20 + i), name, state, Some("run the migration on staging?"), 30 + i as u64 * 60));
            }
        }
        _ => {}
    }
    agents
}

/// A window with a line of text in it
fn terminal(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, title: &str) {
    c.fill(x, y, w, h, opaque(0x141414));
    c.text(x + 14, y + 12, crate::font::Size::Ui, title, 0xa8a8a8);
}

/// A canvas over a band of rows of `c`, as wide as it
fn sub<'a>(c: &'a mut Canvas, _x: i32, y: i32, w: i32, h: i32) -> Canvas<'a> {
    Canvas::new(&mut c.px[(y * c.w) as usize..((y + h) * c.w) as usize], w, h)
}

/// Premultiplied `src` over the screen at (x, y)
fn over(c: &mut Canvas, src: &[u32], x: i32, y: i32, w: i32, h: i32) {
    for row in 0..h {
        for col in 0..w {
            let s = src[(row * w + col) as usize];
            let a = s >> 24;
            if a == 0 || x + col >= c.w || y + row >= c.h {
                continue;
            }
            let d = &mut c.px[((y + row) * c.w + x + col) as usize];
            let ch = |shift: u32| ((s >> shift & 0xff) + (*d >> shift & 0xff) * (255 - a) / 255).min(255) << shift;
            *d = 0xff00_0000 | ch(16) | ch(8) | ch(0);
        }
    }
}

fn load_ppm(path: &str, px: &mut [u32], w: i32, h: i32) {
    let Ok(data) = fs::read(path) else {
        eprintln!("lookout: can't read {path}");
        exit(1);
    };
    // "P6 <w> <h> 255" then a whitespace byte and RGB
    let mut fields = Vec::new();
    let mut pos = 0;
    while fields.len() < 4 {
        while data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let start = pos;
        while !data[pos].is_ascii_whitespace() {
            pos += 1;
        }
        fields.push(String::from_utf8_lossy(&data[start..pos]).to_string());
    }
    let (pw, ph): (i32, i32) = (fields[1].parse().unwrap_or(0), fields[2].parse().unwrap_or(0));
    let rgb = &data[pos + 1..];
    for y in 0..h.min(ph) {
        for x in 0..w.min(pw) {
            let i = ((y * pw + x) * 3) as usize;
            px[(y * w + x) as usize] = 0xff00_0000 | (rgb[i] as u32) << 16 | (rgb[i + 1] as u32) << 8 | rgb[i + 2] as u32;
        }
    }
}

/// An uncompressed PNG: big, but needs nothing
fn save(path: &str, px: &[u32], w: i32, h: i32) {
    let mut raw = Vec::with_capacity((w * h * 3 + h) as usize);
    for row in px.chunks_exact(w as usize) {
        raw.push(0);
        for p in row {
            raw.extend([(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        let len = block.len() as u16;
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(*block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend((b << 16 | a).to_be_bytes());

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        png.extend((data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend(kind);
        png.extend(data);
        png.extend(crc(&png[start..]).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    if let Err(err) = fs::write(path, png) {
        eprintln!("lookout: can't write {path}: {err}");
        exit(1);
    }
}

fn crc(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xedb8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}
