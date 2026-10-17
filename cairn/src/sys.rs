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
    fn tzset();
    fn localtime_r(time: *const i64, tm: *mut Tm) -> *mut Tm;
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
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

/// Local (hour, minute), following the system timezone
pub fn local_time() -> (i32, i32) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let mut tm: Tm = unsafe { std::mem::zeroed() };
    unsafe {
        tzset();
        localtime_r(&now, &mut tm);
    }
    (tm.hour, tm.min)
}
