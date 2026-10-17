//! Minimal Wayland client: messages are 32-bit words over a unix socket, see
//! https://wayland.freedesktop.org/docs/html/ch04.html#sect-Protocol-Wire-Format

use std::env;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::exit;

use crate::sys;

/// wl_display is always object 1
pub const DISPLAY: u32 = 1;

pub enum Arg<'a> {
    Uint(u32),
    Int(i32),
    Str(&'a str),
    /// Object id, 0 for null
    Obj(u32),
}

pub struct Event {
    pub obj: u32,
    pub op: u16,
    data: Vec<u8>,
}

impl Event {
    pub fn args(&self) -> Args<'_> {
        Args { data: &self.data, pos: 0 }
    }
}

pub struct Args<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Args<'_> {
    pub fn uint(&mut self) -> u32 {
        let value = self.data.get(self.pos..self.pos + 4).map_or(0, |b| u32::from_ne_bytes(b.try_into().unwrap()));
        self.pos += 4;
        value
    }

    pub fn string(&mut self) -> String {
        // Length includes the NUL terminator, contents are padded to 4 bytes
        let len = self.uint() as usize;
        let bytes = self.data.get(self.pos..self.pos + len.saturating_sub(1)).unwrap_or_default();
        self.pos += len.next_multiple_of(4);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

pub struct Conn {
    sock: UnixStream,
    inbuf: Vec<u8>,
    next_id: u32,
}

impl Conn {
    pub fn connect() -> io::Result<Conn> {
        let runtime = env::var("XDG_RUNTIME_DIR").map_err(|_| io::Error::other("XDG_RUNTIME_DIR not set"))?;
        let display = env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
        // An absolute WAYLAND_DISPLAY replaces the runtime dir
        let path = PathBuf::from(runtime).join(display);
        Ok(Conn { sock: UnixStream::connect(path)?, inbuf: Vec::new(), next_id: DISPLAY + 1 })
    }

    pub fn fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }

    pub fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn send(&mut self, obj: u32, op: u16, args: &[Arg]) {
        if let Err(err) = self.sock.write_all(&encode(obj, op, args)) {
            lost(err);
        }
    }

    pub fn send_fd(&mut self, obj: u32, op: u16, args: &[Arg], fd: RawFd) {
        if let Err(err) = sys::send_with_fd(self.sock.as_raw_fd(), &encode(obj, op, args), fd) {
            lost(err);
        }
    }

    /// Blocks until something arrives, then returns every complete message
    pub fn read_events(&mut self) -> Vec<Event> {
        let mut buf = [0u8; 4096];
        match self.sock.read(&mut buf) {
            Ok(0) => lost(io::Error::other("compositor closed the connection")),
            Ok(n) => self.inbuf.extend_from_slice(&buf[..n]),
            Err(err) => lost(err),
        }

        let mut events = Vec::new();
        let mut pos = 0;
        while self.inbuf.len() - pos >= 8 {
            let header = word(&self.inbuf, pos + 4);
            let size = (header >> 16) as usize;
            if size < 8 || self.inbuf.len() - pos < size {
                break;
            }
            events.push(Event {
                obj: word(&self.inbuf, pos),
                op: header as u16,
                data: self.inbuf[pos + 8..pos + size].to_vec(),
            });
            pos += size;
        }
        self.inbuf.drain(..pos);
        events
    }
}

fn word(bytes: &[u8], pos: usize) -> u32 {
    u32::from_ne_bytes(bytes[pos..pos + 4].try_into().unwrap())
}

fn encode(obj: u32, op: u16, args: &[Arg]) -> Vec<u8> {
    let mut msg = vec![0u8; 8];
    for arg in args {
        match *arg {
            Arg::Uint(v) | Arg::Obj(v) => msg.extend(v.to_ne_bytes()),
            Arg::Int(v) => msg.extend(v.to_ne_bytes()),
            Arg::Str(s) => {
                msg.extend((s.len() as u32 + 1).to_ne_bytes());
                msg.extend(s.as_bytes());
                msg.push(0);
                msg.resize(msg.len().next_multiple_of(4), 0);
            }
        }
    }
    let header = (msg.len() as u32) << 16 | op as u32;
    msg[0..4].copy_from_slice(&obj.to_ne_bytes());
    msg[4..8].copy_from_slice(&header.to_ne_bytes());
    msg
}

fn lost(err: io::Error) -> ! {
    eprintln!("cairn: lost wayland connection: {err}");
    exit(1);
}
