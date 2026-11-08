//! lookout: a bar across the top of the screen with every coding agent running in herdr-core (`hc`), here and on
//! other machines, the ones that want you first, and the clock and stats. Wayland is spoken directly over its
//! socket, so it works on any compositor with wlr-layer-shell; the agent commands drive Hyprland.
//!
//!   lookout                 the bar (start it with Hyprland). Run again, it replaces the one running
//!   lookout pick            the sheet: every agent, and a search to go to any terminal or start one
//!   lookout new             start an agent in a project on any machine
//!   lookout next            focus the agent that wants you most
//!   lookout kill            end the agent in the focused window
//!   lookout view <address>  what an agent's foot window runs
//!   lookout waiting         Claude Code's Stop hook: label the terminal with background tasks still running
//!   lookout shot <out.png>  draw the bar with made-up agents into a picture (built with --features shot)

mod agents;
mod cmd;
mod desktop;
mod draw;
mod font;
mod gpu;
mod hc;
mod hypr;
mod keys;
mod picker;
#[cfg(feature = "shot")]
mod shot;
mod stats;
mod sys;
mod ui;
mod waiting;
mod wayland;

use std::env;
use std::process::exit;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => desktop::run(),
        ["pick"] => cmd::ask(cmd::PICK),
        ["new"] => cmd::ask(cmd::NEW),
        ["next"] => cmd::next(),
        ["kill"] => cmd::ask(cmd::END),
        ["view", addr] => cmd::view(addr),
        ["waiting"] => waiting::run(),
        #[cfg(feature = "shot")]
        ["shot", ..] => shot::run(&args[1..]),
        _ => {
            eprintln!("usage: lookout [pick|new|next|kill|view <address>|waiting]");
            exit(2);
        }
    }
}
