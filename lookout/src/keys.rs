//! The keyboard, for typing into the picker: what a key types comes from libxkbcommon, loaded at runtime like the
//! NVIDIA library so there's nothing to build against (it comes with any Wayland compositor). Without it the picker
//! still moves and picks, the keys for that are known by their codes

use std::ffi::{c_char, c_int, c_void, CStr};

use crate::sys;

/// Linux key codes (input-event-codes.h) for the keys that move and pick, the same on every layout
pub const ESC: u32 = 1;
pub const BACKSPACE: u32 = 14;
pub const TAB: u32 = 15;
pub const ENTER: u32 = 28;
pub const KP_ENTER: u32 = 96;
pub const UP: u32 = 103;
pub const PAGE_UP: u32 = 104;
pub const DOWN: u32 = 108;
pub const PAGE_DOWN: u32 = 109;
pub const SUPER_L: u32 = 125;
pub const SUPER_R: u32 = 126;

type Ptr = *mut c_void;

struct Xkb {
    context: Ptr,
    keymap_new: unsafe extern "C" fn(Ptr, *const c_char, c_int, c_int) -> Ptr,
    keymap_unref: unsafe extern "C" fn(Ptr),
    state_new: unsafe extern "C" fn(Ptr) -> Ptr,
    state_unref: unsafe extern "C" fn(Ptr),
    update_mask: unsafe extern "C" fn(Ptr, u32, u32, u32, u32, u32, u32) -> c_int,
    utf8: unsafe extern "C" fn(Ptr, u32, *mut c_char, usize) -> c_int,
    mod_active: unsafe extern "C" fn(Ptr, *const c_char, c_int) -> c_int,
    repeats: unsafe extern "C" fn(Ptr, u32) -> c_int,
}

pub struct Keys {
    xkb: Option<Xkb>,
    keymap: Ptr,
    state: Ptr,
}

impl Keys {
    pub fn new() -> Keys {
        Keys { xkb: load(), keymap: std::ptr::null_mut(), state: std::ptr::null_mut() }
    }

    /// The compositor's keymap, as text
    pub fn set_keymap(&mut self, text: &[u8]) {
        let Some(xkb) = &self.xkb else { return };
        let Ok(text) = CStr::from_bytes_until_nul(text).map(CStr::to_owned).or_else(|_| std::ffi::CString::new(text.to_vec())) else { return };
        const TEXT_V1: c_int = 1;
        let keymap = unsafe { (xkb.keymap_new)(xkb.context, text.as_ptr(), TEXT_V1, 0) };
        if keymap.is_null() {
            return;
        }
        let state = unsafe { (xkb.state_new)(keymap) };
        self.drop_keymap();
        (self.keymap, self.state) = (keymap, state);
    }

    pub fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        if let (Some(xkb), false) = (&self.xkb, self.state.is_null()) {
            unsafe { (xkb.update_mask)(self.state, depressed, latched, locked, 0, 0, group) };
        }
    }

    /// What `key` types, empty for keys that type nothing
    pub fn text(&self, key: u32) -> String {
        let Some(xkb) = self.xkb.as_ref().filter(|_| !self.state.is_null()) else { return String::new() };
        let mut buf = [0 as c_char; 16];
        // xkb's codes are Linux's plus 8, from X
        let n = unsafe { (xkb.utf8)(self.state, key + 8, buf.as_mut_ptr(), buf.len()) };
        let bytes: Vec<u8> = buf[..n.clamp(0, 15) as usize].iter().map(|&b| b as u8).collect();
        String::from_utf8(bytes).unwrap_or_default().chars().filter(|c| !c.is_control()).collect()
    }

    /// Whether a modifier is held: "Control", "Mod4" (SUPER)
    pub fn held(&self, name: &CStr) -> bool {
        const EFFECTIVE: c_int = 1 << 3;
        let Some(xkb) = self.xkb.as_ref().filter(|_| !self.state.is_null()) else { return false };
        unsafe { (xkb.mod_active)(self.state, name.as_ptr(), EFFECTIVE) == 1 }
    }

    /// Whether holding `key` repeats it. Without a keymap, only the ones that move and delete
    pub fn repeats(&self, key: u32) -> bool {
        match self.xkb.as_ref().filter(|_| !self.keymap.is_null()) {
            Some(xkb) => unsafe { (xkb.repeats)(self.keymap, key + 8) == 1 },
            None => matches!(key, BACKSPACE | UP | DOWN | PAGE_UP | PAGE_DOWN),
        }
    }

    fn drop_keymap(&mut self) {
        if let Some(xkb) = &self.xkb {
            if !self.state.is_null() {
                unsafe { (xkb.state_unref)(self.state) };
            }
            if !self.keymap.is_null() {
                unsafe { (xkb.keymap_unref)(self.keymap) };
            }
        }
    }
}

fn load() -> Option<Xkb> {
    let lib = sys::dlopen_lib(c"libxkbcommon.so.0")?;
    // The signatures are xkbcommon's (xkbcommon.h)
    unsafe {
        let context_new: unsafe extern "C" fn(c_int) -> Ptr = sys::symbol(lib, c"xkb_context_new")?;
        let context = context_new(0);
        if context.is_null() {
            return None;
        }
        Some(Xkb {
            context,
            keymap_new: sys::symbol(lib, c"xkb_keymap_new_from_string")?,
            keymap_unref: sys::symbol(lib, c"xkb_keymap_unref")?,
            state_new: sys::symbol(lib, c"xkb_state_new")?,
            state_unref: sys::symbol(lib, c"xkb_state_unref")?,
            update_mask: sys::symbol(lib, c"xkb_state_update_mask")?,
            utf8: sys::symbol(lib, c"xkb_state_key_get_utf8")?,
            mod_active: sys::symbol(lib, c"xkb_state_mod_name_is_active")?,
            repeats: sys::symbol(lib, c"xkb_keymap_key_repeats")?,
        })
    }
}
