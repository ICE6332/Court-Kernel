#![no_std]
#![no_main]

use core::sync::atomic::Ordering;
use court_abi::{
    BOOT_MAGIC, BOOT_VA, BootInfo, CAPACITY, RIGHT_RECV, RING_MAGIC, RING_VA, Ring, ST_BAD_MAGIC,
    ST_DENIED, ST_EMPTY, ST_RECV, slot_ptr,
};

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> u64 {
    let boot = unsafe { &*(BOOT_VA as *const BootInfo) };
    if boot.magic != BOOT_MAGIC {
        return ST_BAD_MAGIC;
    }
    let ring = unsafe { &*(RING_VA as *const Ring) };
    if ring.magic != RING_MAGIC {
        return ST_BAD_MAGIC;
    }
    if boot.rights & RIGHT_RECV == 0 || ring.revoked.load(Ordering::Acquire) != 0 {
        return ST_DENIED;
    }
    let prod = ring.producer.load(Ordering::Acquire);
    let cons = ring.consumer.load(Ordering::Relaxed);
    if cons == prod {
        return ST_EMPTY;
    }
    let idx = cons % u64::from(CAPACITY);
    let len = unsafe { slot_ptr(idx).cast::<u32>().read() as u64 };
    ring.consumer.store(cons + 1, Ordering::Release);
    ST_RECV | (len << 8)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") }
    }
}
