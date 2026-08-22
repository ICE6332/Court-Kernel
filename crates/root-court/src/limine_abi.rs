//! Minimal Limine boot protocol requests.
//!
//! Implemented locally so Root Court stays on stable Rust 1.98. The layout
//! matches Limine protocol IDs used by the v10.x bootloader.

use core::ffi::CStr;
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

const COMMON_MAGIC: [u64; 2] = [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b];

#[repr(C)]
pub struct BaseRevision {
    magic: [u64; 2],
    revision: u64,
}

impl BaseRevision {
    pub const fn new(revision: u64) -> Self {
        Self {
            magic: [0xf9562b2d5c95a6c8, 0x6a7f439829d2dc32],
            revision,
        }
    }

    pub fn loaded_revision(&self) -> u64 {
        unsafe { core::ptr::addr_of!(self.revision).read_volatile() }
    }

    pub fn is_supported(&self) -> bool {
        self.loaded_revision() == 0
    }
}

#[repr(C)]
pub struct RequestsStartMarker {
    _marker: [u64; 4],
}

impl RequestsStartMarker {
    pub const fn new() -> Self {
        Self {
            _marker: [
                0xf6b8f4b39de7d1ae,
                0xfab91a8d6e8c0f23,
                0x06c0397da013746c,
                0x13d86c035a1cd3e1,
            ],
        }
    }
}

#[repr(C)]
pub struct RequestsEndMarker {
    _marker: [u64; 2],
}

impl RequestsEndMarker {
    pub const fn new() -> Self {
        Self {
            _marker: [0xadc0e0531bb10d03, 0x9572709f31764c62],
        }
    }
}

#[repr(C)]
struct Request<Extra, Resp> {
    id: [u64; 4],
    revision: u64,
    response: AtomicPtr<Resp>,
    extra: Extra,
}

impl<Extra, Resp> Request<Extra, Resp> {
    const fn new(id: [u64; 2], extra: Extra) -> Self {
        Self {
            id: [COMMON_MAGIC[0], COMMON_MAGIC[1], id[0], id[1]],
            revision: 0,
            response: AtomicPtr::new(core::ptr::null_mut()),
            extra,
        }
    }

    fn response(&self) -> Option<&'static Resp> {
        let ptr = self.response.load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { &*ptr })
        }
    }
}

#[repr(C)]
pub struct BootloaderInfoResponse {
    pub revision: u64,
    name: *const core::ffi::c_char,
    version: *const core::ffi::c_char,
}

impl BootloaderInfoResponse {
    pub fn name(&self) -> &str {
        unsafe { CStr::from_ptr(self.name).to_str().unwrap_or("?") }
    }

    pub fn version(&self) -> &str {
        unsafe { CStr::from_ptr(self.version).to_str().unwrap_or("?") }
    }
}

pub struct BootloaderInfoRequest(Request<(), BootloaderInfoResponse>);

impl BootloaderInfoRequest {
    pub const fn new() -> Self {
        Self(Request::new([0xf55038d8e2a1202f, 0x279426fcf5f59740], ()))
    }

    pub fn response(&self) -> Option<&'static BootloaderInfoResponse> {
        self.0.response()
    }
}

#[repr(C)]
pub struct FirmwareTypeResponse {
    pub revision: u64,
    pub firmware_type: u64,
}

pub struct FirmwareTypeRequest(Request<(), FirmwareTypeResponse>);

impl FirmwareTypeRequest {
    pub const fn new() -> Self {
        Self(Request::new([0x8c2f75d90bef28a8, 0x7045a4688eac00c3], ()))
    }

    pub fn response(&self) -> Option<&'static FirmwareTypeResponse> {
        self.0.response()
    }
}

#[repr(C)]
pub struct HhdmResponse {
    pub revision: u64,
    pub offset: u64,
}

pub struct HhdmRequest(Request<(), HhdmResponse>);

impl HhdmRequest {
    pub const fn new() -> Self {
        Self(Request::new([0x48dcf1cb8ad2b852, 0x63984e959a98244b], ()))
    }

    pub fn response(&self) -> Option<&'static HhdmResponse> {
        self.0.response()
    }
}

#[repr(C)]
pub struct StackSizeRequest(Request<u64, StackSizeResponse>);

#[repr(C)]
pub struct StackSizeResponse {
    pub revision: u64,
}

impl StackSizeRequest {
    pub const fn new(size: u64) -> Self {
        Self(Request::new([0x224ef0460a8e8926, 0xe1cb0fc25f46ea3d], size))
    }

    pub fn response(&self) -> Option<&'static StackSizeResponse> {
        self.0.response()
    }
}

pub const MEMMAP_USABLE: u64 = 0;

#[repr(C)]
pub struct MemmapEntry {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

#[repr(C)]
pub struct MemmapResponse {
    pub revision: u64,
    entry_count: u64,
    entries: *const *const MemmapEntry,
}

impl MemmapResponse {
    pub fn entries(&self) -> &[&MemmapEntry] {
        unsafe { core::slice::from_raw_parts(self.entries.cast(), self.entry_count as usize) }
    }
}

pub struct MemmapRequest(Request<(), MemmapResponse>);

impl MemmapRequest {
    pub const fn new() -> Self {
        Self(Request::new([0x67cf3d9d378a806f, 0xe304acdfc50c3c62], ()))
    }

    pub fn response(&self) -> Option<&'static MemmapResponse> {
        self.0.response()
    }
}

pub const MP_X2APIC: u64 = 1;

#[repr(C)]
pub struct MpInfo {
    pub processor_id: u32,
    pub lapic_id: u32,
    _reserved: u64,
    goto_address: AtomicU64,
    pub extra_argument: u64,
}

impl MpInfo {
    pub fn start(&self, entry: extern "C" fn(&'static MpInfo) -> !) {
        self.goto_address
            .store(entry as usize as u64, Ordering::Release);
    }
}

#[repr(C)]
pub struct MpResponse {
    pub revision: u64,
    pub flags: u32,
    pub bsp_lapic_id: u32,
    cpu_count: u64,
    cpus: *const *const MpInfo,
}

impl MpResponse {
    pub fn cpus(&self) -> &[&MpInfo] {
        unsafe { core::slice::from_raw_parts(self.cpus.cast(), self.cpu_count as usize) }
    }
}

pub struct MpRequest(Request<u64, MpResponse>);

impl MpRequest {
    pub const fn new(flags: u64) -> Self {
        Self(Request::new(
            [0x95a67b819a1b857e, 0xa0b61b723b6a73e0],
            flags,
        ))
    }

    pub fn response(&self) -> Option<&'static MpResponse> {
        self.0.response()
    }
}

pub const FIRMWARE_UEFI64: u64 = 2;
