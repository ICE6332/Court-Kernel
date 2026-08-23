#![no_std]
#![no_main]

use core::sync::atomic::Ordering;
use court_abi::{
    BOOT_MAGIC, BOOT_VA, BootInfo, CAPACITY, RIGHT_SEND, RING_MAGIC, RING_VA, Ring, ST_BAD_MAGIC,
    ST_DENIED, ST_SENT, slot_ptr,
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
    if boot.rights & RIGHT_SEND == 0 || ring.revoked.load(Ordering::Acquire) != 0 {
        return ST_DENIED;
    }
    let prod = ring.producer.load(Ordering::Relaxed);
    let cons = ring.consumer.load(Ordering::Acquire);
    if prod.wrapping_sub(cons) >= u64::from(CAPACITY) {
        return ST_DENIED;
    }
    let idx = prod % u64::from(CAPACITY);
    let payload = b"pkt";
    unsafe {
        slot_ptr(idx).cast::<u32>().write(payload.len() as u32);
        slot_ptr(idx)
            .add(4)
            .copy_from_nonoverlapping(payload.as_ptr(), payload.len());
    }
    ring.producer.store(prod + 1, Ordering::Release);
    ST_SENT
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("hlt") }
    }
}
