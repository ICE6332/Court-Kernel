//! Bring-up layouts shared by Root Court and Court Images.
//!
//! This is not a protocol stack, filesystem, or syscall ABI. It is the
//! minimum C layout a trusted Court Image needs to find its boot block
//! and shared ring after Root Court has mapped them.

#![no_std]

use core::sync::atomic::AtomicU64;

pub const PAGE: u64 = 0x1000;
pub const IMAGE_VA: u64 = 0x0010_0000;
pub const BOOT_VA: u64 = 0x0020_0000;
pub const RING_VA: u64 = 0x0030_0000;
pub const STACK_VA: u64 = 0x0040_0000;
pub const STACK_PAGES: u64 = 8;
pub const STACK_SIZE: u64 = STACK_PAGES * PAGE;

pub const BOOT_MAGIC: u64 = 0x434B_424F_4F54_3032; // CKBOOT02
pub const RING_MAGIC: u64 = 0x434B_5249_4E47_3031; // CKRING01

pub const SLOT: usize = 64;
pub const CAPACITY: u32 = 4;
pub const RING_DATA: usize = 128;

pub const RIGHT_SEND: u64 = 1 << 0;
pub const RIGHT_RECV: u64 = 1 << 1;

pub const ST_SENT: u64 = 0x0001;
pub const ST_RECV: u64 = 0x0002;
pub const ST_DENIED: u64 = 0xE001;
pub const ST_EMPTY: u64 = 0xE002;
pub const ST_BAD_MAGIC: u64 = 0xBAD0;

#[repr(C)]
pub struct BootInfo {
    pub magic: u64,
    pub cap_id: u64,
    pub rights: u64,
    pub ring_virt: u64,
    pub court_id: u64,
}

#[repr(C)]
pub struct Ring {
    pub magic: u64,
    pub capacity: u32,
    pub slot_size: u32,
    pub producer: AtomicU64,
    pub consumer: AtomicU64,
    pub revoked: AtomicU64,
}

#[inline]
pub fn slot_ptr(index: u64) -> *mut u8 {
    (RING_VA + RING_DATA as u64 + index * SLOT as u64) as *mut u8
}
