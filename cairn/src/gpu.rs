//! The NVIDIA GPU through NVML, loaded at runtime so cairn still runs on machines without the driver

use std::ffi::{c_int, c_void};
use std::path::Path;

use crate::sys;

#[repr(C)]
#[derive(Default)]
struct Utilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct Memory {
    total: u64,
    free: u64,
    used: u64,
}

type Device = *mut c_void;

pub struct Gpu {
    device: Device,
    utilization: extern "C" fn(Device, *mut Utilization) -> c_int,
    memory: extern "C" fn(Device, *mut Memory) -> c_int,
}

impl Gpu {
    /// The first NVIDIA GPU, or None if the driver isn't loaded
    pub fn open() -> Option<Gpu> {
        // Checked first so a machine without the driver never loads the library
        if !Path::new("/proc/driver/nvidia").exists() {
            return None;
        }
        let lib = sys::dlopen_lib(c"libnvidia-ml.so.1")?;
        unsafe {
            let init: extern "C" fn() -> c_int = std::mem::transmute(sys::dlsym_fn(lib, c"nvmlInit_v2")?);
            let by_index: extern "C" fn(u32, *mut Device) -> c_int =
                std::mem::transmute(sys::dlsym_fn(lib, c"nvmlDeviceGetHandleByIndex_v2")?);
            let utilization = std::mem::transmute(sys::dlsym_fn(lib, c"nvmlDeviceGetUtilizationRates")?);
            let memory = std::mem::transmute(sys::dlsym_fn(lib, c"nvmlDeviceGetMemoryInfo")?);
            let mut device = std::ptr::null_mut();
            if init() != 0 || by_index(0, &mut device) != 0 {
                return None;
            }
            Some(Gpu { device, utilization, memory })
        }
    }

    /// Percent busy over the driver's last sample period
    pub fn busy(&self) -> Option<u32> {
        let mut u = Utilization::default();
        ((self.utilization)(self.device, &mut u) == 0).then_some(u.gpu)
    }

    /// (used, total) video memory in GiB
    pub fn memory(&self) -> Option<(f32, f32)> {
        let mut m = Memory::default();
        let gib = |b: u64| b as f32 / 1024.0 / 1024.0 / 1024.0;
        ((self.memory)(self.device, &mut m) == 0).then(|| (gib(m.used), gib(m.total)))
    }
}
