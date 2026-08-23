//! Per-CPU kernel stacks. Limine stacks live in bootloader-reclaimable
//! memory; reclaim is still deferred, but every CPU leaves that stack.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::cpu::MAX_CPUS;
use crate::mm::BumpAllocator;

pub const STACK_PAGES: u64 = 16; // 64 KiB
pub const STACK_SIZE: u64 = STACK_PAGES * 0x1000;

static TOPS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

pub fn alloc_all(bump: &mut BumpAllocator, ncpu: usize) -> Result<(), &'static str> {
    if ncpu == 0 || ncpu > MAX_CPUS {
        return Err("cpu count for kernel stacks");
    }
    for cpu in 0..ncpu {
        let (_phys, virt) = bump
            .alloc_pages(STACK_PAGES)
            .ok_or("kernel stack alloc failed")?;
        // SAFETY: freshly reserved usable pages, exclusive to this CPU.
        unsafe { virt.write_bytes(0, STACK_SIZE as usize) };
        let top = (virt as u64).saturating_add(STACK_SIZE) & !0xF;
        TOPS[cpu].store(top, Ordering::Release);
    }
    Ok(())
}

pub fn top(cpu: usize) -> u64 {
    if cpu >= MAX_CPUS {
        return 0;
    }
    TOPS[cpu].load(Ordering::Acquire)
}

/// Switch onto `new_rsp` and jump to `cont(arg)`. Does not return.
///
/// `new_rsp` must be 16-byte aligned. A dummy return address is pushed so the
/// callee sees the SysV `rsp % 16 == 8` entry alignment.
pub unsafe fn switch_to(new_rsp: u64, cont: extern "C" fn(u64) -> !, arg: u64) -> ! {
    unsafe {
        asm!(
            "mov rsp, {rsp}",
            "xor rbp, rbp",
            "push 0",
            "jmp {cont}",
            rsp = in(reg) new_rsp,
            cont = in(reg) cont,
            in("rdi") arg,
            options(noreturn),
        );
    }
}
