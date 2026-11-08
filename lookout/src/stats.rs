//! Everything is read straight from /sys and /proc

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::gpu::Gpu;
use crate::sys;

/// Reading the battery goes through ACPI to the embedded controller, ~15ms each time
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// Asks the wifi driver, ~1ms, which was most of each 5s tick's work when it ran every tick
const WIFI_EVERY: Duration = Duration::from_secs(30);

/// Label, value, and an optional note shown quieter after it
pub type Item = (&'static str, String, Option<String>);

pub struct Stats {
    battery: Option<(PathBuf, Slow)>,
    wireless: Option<Slow>,
    cpu_prev: Option<(u64, u64)>,
    gpu: Option<Gpu>,
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
            cpu_prev: None,
            gpu: Gpu::open(),
        }
    }

    /// The time, "14:32", and the date, "Sat 7 Nov"
    pub fn clock(&self) -> (String, String) {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let t = sys::local_time();
        let date = format!("{} {} {}", DAYS[t.weekday.rem_euclid(7) as usize], t.day, MONTHS[t.month.rem_euclid(12) as usize]);
        (format!("{:02}:{:02}", t.hour, t.min), date)
    }

    /// Each stat. While charging the battery's label is CHG rather than BAT
    pub fn items(&mut self) -> Vec<Item> {
        let mut items = Vec::new();
        if let Some((path, slow)) = &mut self.battery {
            // "<percent>" or "<percent>c" while charging, read together since both are slow
            let reading = slow.get(|| {
                let capacity = fs::read_to_string(path.join("capacity")).unwrap_or_default();
                let charging = fs::read_to_string(path.join("status")).is_ok_and(|s| s.trim() == "Charging");
                format!("{}{}", capacity.trim(), if charging { "c" } else { "" })
            });
            let charging = reading.ends_with('c');
            items.push((if charging { "CHG" } else { "BAT" }, reading.trim_end_matches('c').to_string(), None));
        }
        if let Some(slow) = &mut self.wireless {
            items.push(("WIFI", slow.get(|| wifi().map_or("-".into(), |q| q.to_string())), None));
        }
        // Not until there are two readings to compare
        if let Some(cpu) = self.cpu() {
            items.push(("CPU", cpu.to_string(), None));
        }
        if let Some((used, total)) = memory() {
            items.push(("MEM", format!("{used:.1}"), Some(capacity(total))));
        }
        if let Some(gpu) = &self.gpu {
            items.push(("GPU", gpu.busy().map_or("-".into(), |b| b.to_string()), None));
            if let Some((used, total)) = gpu.memory() {
                items.push(("VRAM", format!("{used:.1}"), Some(capacity(total))));
            }
        }
        items
    }

    fn cpu(&mut self) -> Option<u32> {
        let (idle, total) = cpu_times()?;
        let (prev_idle, prev_total) = self.cpu_prev.replace((idle, total))?;
        let total = total.saturating_sub(prev_total);
        let busy = total.saturating_sub(idle.saturating_sub(prev_idle));
        Some((busy * 100).checked_div(total).unwrap_or(0) as u32)
    }
}

/// Total memory as a whole number of GiB after a slash, e.g. "/16", so "3.6/16" reads as used of total
fn capacity(total: f32) -> String {
    format!("/{}", total.round())
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
