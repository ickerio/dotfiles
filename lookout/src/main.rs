//! lookout: the clock and stats written quietly onto the desktop in the sky left of the wallpaper's mountain, and below
//! them every coding agent running in herdr-core (`hc`), here and on other machines. Wayland is spoken directly over
//! its socket, so it works on any compositor with wlr-layer-shell; the agent commands drive Hyprland and fuzzel.
//!
//!   lookout                 the board (start it with Hyprland)
//!   lookout peek|unpeek     raise the board over the windows, or put it back
//!   lookout pick            every terminal in fuzzel: focus its window or open one
//!   lookout new             start an agent in a project on any machine
//!   lookout next            focus the agent that wants you most
//!   lookout kill            end the agent in the focused window
//!   lookout view <address>  what an agent's foot window runs
//!   lookout waiting         Claude Code's Stop hook: label the terminal with background tasks still running

mod agents;
mod cmd;
mod desktop;
mod font;
mod gpu;
mod hc;
mod hypr;
mod stats;
mod sys;
mod waiting;
mod wayland;

use std::env;
use std::process::exit;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => desktop::run(),
        ["peek"] => cmd::signal(sys::SIGUSR1),
        ["unpeek"] => cmd::signal(sys::SIGUSR2),
        ["pick"] => cmd::pick(),
        ["new"] => cmd::new(),
        ["next"] => cmd::next(),
        ["kill"] => cmd::kill(),
        ["view", addr] => cmd::view(addr),
        ["waiting"] => waiting::run(),
        _ => {
            eprintln!("usage: lookout [peek|unpeek|pick|new|next|kill|view <address>|waiting]");
            exit(2);
        }
    }
}
