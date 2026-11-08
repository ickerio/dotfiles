//! The few libc calls std doesn't expose, declared by hand instead of pulling in the libc crate

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::raw::{c_char, c_int, c_ulong, c_void};
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C)]
pub struct PollFd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

pub const POLLIN: i16 = 1;

#[repr(C)]
struct IoVec {
    base: *const c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    namelen: u32,
    iov: *const IoVec,
    iovlen: usize,
    control: *const c_void,
    controllen: usize,
    flags: c_int,
}

#[repr(C)]
struct Tm {
    sec: c_int,
    min: c_int,
    hour: c_int,
    mday: c_int,
    mon: c_int,
    year: c_int,
    wday: c_int,
    yday: c_int,
    isdst: c_int,
    gmtoff: i64,
    zone: *const c_char,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
    fn memfd_create(name: *const c_char, flags: u32) -> c_int;
    fn sendmsg(fd: c_int, msg: *const MsgHdr, flags: c_int) -> isize;
    fn recvmsg(fd: c_int, msg: *mut MsgHdr, flags: c_int) -> isize;
    fn tzset();
    fn localtime_r(time: *const i64, tm: *mut Tm) -> *mut Tm;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn sigemptyset(set: *mut SigSet) -> c_int;
    fn sigaddset(set: *mut SigSet, signum: c_int) -> c_int;
    fn sigprocmask(how: c_int, set: *const SigSet, old: *mut SigSet) -> c_int;
    fn signalfd(fd: c_int, mask: *const SigSet, flags: c_int) -> c_int;
    fn sigqueue(pid: c_int, sig: c_int, value: *mut c_void) -> c_int;
}

/// glibc's sigset_t, 1024 bits
#[repr(C)]
struct SigSet([u64; 16]);

pub const SIGTERM: c_int = 15;
/// What `lookout pick`, `new` and `kill` send, with which in its value. A real-time signal, so each one sent arrives
/// rather than ones close together merging. SIGRTMIN is 34 under glibc, and this is clear of what anything else uses
pub const SIGCMD: c_int = 40;

/// Blocks `signals` and returns a file that becomes readable when one arrives, so they're handled in the poll
/// loop rather than in a handler. Children started by std::process get the mask cleared again
pub fn signal_file(signals: &[c_int]) -> io::Result<File> {
    let mut set = SigSet([0; 16]);
    unsafe {
        sigemptyset(&mut set);
        for &signal in signals {
            sigaddset(&mut set, signal);
        }
        const SIG_BLOCK: c_int = 0;
        if sigprocmask(SIG_BLOCK, &set, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
        const SFD_NONBLOCK_CLOEXEC: c_int = 0o4000 | 0o2000000;
        let fd = signalfd(-1, &set, SFD_NONBLOCK_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(fd))
    }
}

/// The signals waiting in a file from signal_file, each with the value it was sent with (0 from a plain kill),
/// empty once it's drained
pub fn read_signals(file: &mut File) -> Vec<(c_int, u64)> {
    use std::io::Read;
    // Each is a 128 byte signalfd_siginfo, starting with the signal number and with sigqueue's value at 48
    let mut buf = [0u8; 128 * 8];
    let n = file.read(&mut buf).unwrap_or(0);
    let word = |info: &[u8], at: usize| u64::from_ne_bytes(info[at..at + 8].try_into().unwrap());
    let (infos, _) = buf[..n].as_chunks::<128>();
    infos.iter().map(|info| (u32::from_ne_bytes(info[..4].try_into().unwrap()) as c_int, word(info, 48))).collect()
}

/// Sends `signal` to `pid` with `value` along with it
pub fn send_signal(pid: i32, signal: c_int, value: u64) -> io::Result<()> {
    if unsafe { sigqueue(pid, signal, value as usize as *mut c_void) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn poll_fds(fds: &mut [PollFd], timeout_ms: i32) -> io::Result<()> {
    if unsafe { poll(fds.as_mut_ptr(), fds.len() as c_ulong, timeout_ms) } < 0 {
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
    Ok(())
}

/// Anonymous in-memory file, shared with the compositor as the pixel buffer
pub fn memfd(name: &CStr) -> io::Result<File> {
    const MFD_CLOEXEC: u32 = 1;
    let fd = unsafe { memfd_create(name.as_ptr(), MFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// `file` mapped into memory as pixels, shared so the compositor sees what's drawn without any writes
pub struct Map {
    ptr: *mut u32,
    len: usize,
}

impl Map {
    pub fn new(file: &File, bytes: usize) -> io::Result<Map> {
        const PROT_READ_WRITE: c_int = 3;
        const MAP_SHARED: c_int = 1;
        let ptr = unsafe { mmap(std::ptr::null_mut(), bytes, PROT_READ_WRITE, MAP_SHARED, file.as_raw_fd(), 0) };
        if ptr as isize == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Map { ptr: ptr.cast(), len: bytes / 4 })
    }

    pub fn pixels(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        unsafe { munmap(self.ptr.cast(), self.len * 4) };
    }
}

/// Sends `data` with `fd` attached (SCM_RIGHTS), which is how Wayland passes file descriptors
pub fn send_with_fd(sock: RawFd, data: &[u8], fd: RawFd) -> io::Result<()> {
    const SOL_SOCKET: c_int = 1;
    const SCM_RIGHTS: c_int = 1;

    // cmsghdr { len: usize, level: c_int, type: c_int } then the fd, padded to 8 bytes
    let mut control = [0u64; 3];
    let bytes = unsafe { &mut *(control.as_mut_ptr() as *mut [u8; 24]) };
    bytes[0..8].copy_from_slice(&20usize.to_ne_bytes());
    bytes[8..12].copy_from_slice(&SOL_SOCKET.to_ne_bytes());
    bytes[12..16].copy_from_slice(&SCM_RIGHTS.to_ne_bytes());
    bytes[16..20].copy_from_slice(&fd.to_ne_bytes());

    let iov = IoVec { base: data.as_ptr().cast(), len: data.len() };
    let msg = MsgHdr {
        name: std::ptr::null_mut(),
        namelen: 0,
        iov: &iov,
        iovlen: 1,
        control: control.as_ptr().cast(),
        controllen: 24,
        flags: 0,
    };
    let sent = unsafe { sendmsg(sock, &msg, 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    if sent as usize != data.len() {
        return Err(io::Error::other("short write on wayland socket"));
    }
    Ok(())
}

/// Reads what's waiting on `sock` into `buf`, with any file descriptors sent along with it (SCM_RIGHTS), which is
/// how Wayland passes them. 0 bytes is the end
pub fn recv_with_fds(sock: RawFd, buf: &mut [u8]) -> io::Result<(usize, Vec<RawFd>)> {
    const SOL_SOCKET: c_int = 1;
    const SCM_RIGHTS: c_int = 1;
    const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
    let mut control = [0u64; 16];
    let iov = IoVec { base: buf.as_mut_ptr().cast(), len: buf.len() };
    let mut msg = MsgHdr {
        name: std::ptr::null_mut(),
        namelen: 0,
        iov: &iov,
        iovlen: 1,
        control: control.as_mut_ptr().cast(),
        controllen: std::mem::size_of_val(&control),
        flags: 0,
    };
    let n = unsafe { recvmsg(sock, &mut msg, MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    // Each cmsghdr is { len: usize, level: c_int, type: c_int } then its data, padded to 8 bytes
    let bytes = unsafe { std::slice::from_raw_parts(control.as_ptr().cast::<u8>(), msg.controllen.min(128)) };
    let mut fds = Vec::new();
    let mut pos = 0;
    while pos + 16 <= bytes.len() {
        let len = usize::from_ne_bytes(bytes[pos..pos + 8].try_into().unwrap());
        let level = c_int::from_ne_bytes(bytes[pos + 8..pos + 12].try_into().unwrap());
        let kind = c_int::from_ne_bytes(bytes[pos + 12..pos + 16].try_into().unwrap());
        if len < 16 || pos + len > bytes.len() {
            break;
        }
        if level == SOL_SOCKET && kind == SCM_RIGHTS {
            let (fd_bytes, _) = bytes[pos + 16..pos + len].as_chunks::<4>();
            fds.extend(fd_bytes.iter().map(|fd| c_int::from_ne_bytes(*fd)));
        }
        pos += len.next_multiple_of(8);
    }
    Ok((n as usize, fds))
}

/// A shared library loaded for the life of the process, None if it isn't installed
pub fn dlopen_lib(name: &CStr) -> Option<*mut c_void> {
    const RTLD_NOW: c_int = 2;
    let lib = unsafe { dlopen(name.as_ptr(), RTLD_NOW) };
    (!lib.is_null()).then_some(lib)
}

/// The function `name` in `lib` as the function pointer type `F`, an `unsafe extern "C" fn(..)`, so calling it
/// takes an `unsafe` block.
///
/// # Safety
/// `F` must be the symbol's real signature
pub unsafe fn symbol<F: Copy>(lib: *mut c_void, name: &CStr) -> Option<F> {
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>(), "F must be a function pointer");
    let sym = unsafe { dlsym(lib, name.as_ptr()) };
    (!sym.is_null()).then(|| unsafe { std::mem::transmute_copy(&sym) })
}

/// Local time, following the system timezone
pub struct Time {
    pub hour: i32,
    pub min: i32,
    /// Day of the month from 1, month from 0, weekday from 0 for Sunday
    pub day: i32,
    pub month: i32,
    pub weekday: i32,
}

pub fn local_time() -> Time {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    unsafe {
        tzset();
        localtime_r(&now, &mut tm);
    }
    Time { hour: tm.hour, min: tm.min, day: tm.mday, month: tm.mon, weekday: tm.wday }
}
