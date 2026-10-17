//! Everything is read straight from /sys and /proc

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::sys;

/// Reading the battery goes through ACPI to the embedded controller, ~15ms each time
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// Asks the wifi driver, ~1.5ms
const WIFI_EVERY: Duration = Duration::from_secs(5);

pub struct Stats {
    battery: Option<(PathBuf, Slow)>,
    wireless: Option<Slow>,
    mem_total: f32,
    cpu_prev: Option<(u64, u64)>,
}

/// A value that's costly to read, kept for a while between readings
struct Slow {
    every: Duration,
    value: String,
    read_at: Option<Instant>,
}

impl Slow {
    fn new(every: Duration) -> Slow {
        Slow { every, value: String::new(), read_at: None }
    }

    fn get(&mut self, read: impl FnOnce() -> String) -> String {
        if self.read_at.is_none_or(|t| t.elapsed() >= self.every) {
            self.value = read();
            self.read_at = Some(Instant::now());
        }
        self.value.clone()
    }
}

impl Stats {
    pub fn new() -> Stats {
        let battery = fs::read_dir("/sys/class/power_supply").ok().and_then(|dir| {
            dir.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("BAT")))
        });
        let wireless = fs::read_dir("/sys/class/net")
            .map(|dir| dir.flatten().any(|e| e.path().join("wireless").exists()))
            .unwrap_or(false);
        Stats {
            battery: battery.map(|path| (path, Slow::new(BATTERY_EVERY))),
            wireless: wireless.then(|| Slow::new(WIFI_EVERY)),
            mem_total: memory().map_or(0.0, |m| m.1),
            cpu_prev: None,
        }
    }

    /// (label, value) pairs, the clock first with an empty label
    pub fn items(&mut self) -> Vec<(&'static str, String)> {
        let (hour, min) = sys::local_time();
        let mut items = vec![("", format!("{hour:02}:{min:02}"))];
        if let Some((path, slow)) = &mut self.battery {
            items.push((
                "BAT",
                slow.get(|| {
                    let capacity = fs::read_to_string(path.join("capacity")).unwrap_or_default();
                    let charging = fs::read_to_string(path.join("status")).is_ok_and(|s| s.trim() == "Charging");
                    format!("{}{}", capacity.trim(), if charging { "+" } else { "" })
                }),
            ));
        }
        if let Some(slow) = &mut self.wireless {
            items.push(("WIFI", slow.get(|| wifi().map_or("-".into(), |q| q.to_string()))));
        }
        items.push(("CPU", self.cpu().map_or("-".into(), |c| c.to_string())));
        if let Some((used, total)) = memory() {
            items.push(("MEM", format!("{used:.1}/{total:.1}")));
        }
        items
    }

    /// The longest values each item can show, used to size the text once so it doesn't jump around
    pub fn widest(&self) -> Vec<(&'static str, String)> {
        let mut items = vec![("", "00:00".into())];
        if self.battery.is_some() {
            items.push(("BAT", "100+".into()));
        }
        if self.wireless.is_some() {
            items.push(("WIFI", "100".into()));
        }
        items.push(("CPU", "100".into()));
        items.push(("MEM", format!("{0:.1}/{0:.1}", self.mem_total)));
        items
    }

    fn cpu(&mut self) -> Option<u32> {
        let (idle, total) = cpu_times()?;
        let (prev_idle, prev_total) = self.cpu_prev.replace((idle, total))?;
        let total = total.saturating_sub(prev_total);
        let busy = total.saturating_sub(idle.saturating_sub(prev_idle));
        Some(if total == 0 { 0 } else { (busy * 100 / total) as u32 })
    }
}

/// (idle, total) jiffies across all CPUs
fn cpu_times() -> Option<(u64, u64)> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    // cpu user nice system idle iowait irq softirq steal ...
    let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).take(8).filter_map(|f| f.parse().ok()).collect();
    if fields.len() < 8 {
        return None;
    }
    Some((fields[3] + fields[4], fields.iter().sum()))
}

/// Link quality of the first wireless interface, out of 70
fn wifi() -> Option<u32> {
    let wireless = fs::read_to_string("/proc/net/wireless").ok()?;
    let link: f32 = wireless.lines().nth(2)?.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
    Some(((link / 70.0) * 100.0).round().min(100.0) as u32)
}

/// (used, total) in GiB, where used excludes cache the kernel can reclaim
fn memory() -> Option<(f32, f32)> {
    let meminfo = fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<f32> {
        let line = meminfo.lines().find(|l| l.starts_with(name))?;
        let kb: f32 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb / 1024.0 / 1024.0)
    };
    let total = field("MemTotal:")?;
    Some((total - field("MemAvailable:")?, total))
}
