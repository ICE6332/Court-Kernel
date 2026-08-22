//! Thin x86-64 primitives. No policy lives here.

use core::arch::asm;
use core::arch::x86_64::{CpuidResult, __cpuid_count, _rdtsc};

pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const KERNEL_TSS: u16 = 0x18;

/// Static per-CPU GDT/TSS slots. QEMU bring-up uses 4; raise before large SMP.
pub const MAX_CPUS: usize = 8;

pub fn cpuid(leaf: u32, subleaf: u32) -> CpuidResult {
    __cpuid_count(leaf, subleaf)
}

pub fn has_x2apic() -> bool {
    cpuid(1, 0).ecx & (1 << 21) != 0
}

pub fn rdtsc() -> u64 {
    unsafe { _rdtsc() }
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: caller names a valid MSR for this CPU mode.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

pub unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    // SAFETY: caller names a valid MSR and value for this CPU mode.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn cli() {
    // SAFETY: clearing IF is a CPU-local flag update.
    unsafe {
        asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

pub unsafe fn sti() {
    // SAFETY: caller has loaded an IDT that can handle maskable IRQs.
    unsafe {
        asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

pub fn hlt() {
    // SAFETY: halt until the next interrupt or NMI; no memory is accessed.
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: caller owns this I/O port.
    unsafe {
        asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: caller owns this I/O port.
    unsafe {
        asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
}

pub fn cr2() -> u64 {
    let value: u64;
    // SAFETY: reading CR2 is always allowed in kernel mode.
    unsafe {
        asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}
