//! x2APIC enable, periodic timer, and ICR IPI.

use crate::cpu::{self, has_x2apic};
use crate::idt::{IPI_VECTOR, SPURIOUS_VECTOR, TIMER_VECTOR};

const IA32_APIC_BASE: u32 = 0x1B;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_BASE_X2APIC: u64 = 1 << 10;

const MSR_APIC_ID: u32 = 0x802;
const MSR_EOI: u32 = 0x80B;
const MSR_SVR: u32 = 0x80F;
const MSR_ICR: u32 = 0x830;
const MSR_LVT_TIMER: u32 = 0x832;
const MSR_TIMER_INIT: u32 = 0x838;
const MSR_TIMER_DIV: u32 = 0x83E;

const SVR_APIC_ENABLE: u64 = 1 << 8;
const LVT_MASK: u64 = 1 << 16;
const LVT_PERIODIC: u64 = 1 << 17;
const TIMER_DIVIDE_BY_1: u64 = 0b1011;
const ICR_DEST_SELF: u64 = 1 << 18;
const ICR_DEST_ALL_EXCL_SELF: u64 = 3 << 18;

const PIC1_DATA: u16 = 0x21;
const PIC2_DATA: u16 = 0xA1;

pub fn mask_legacy_pic() {
    // SAFETY: masking the 8259s prevents legacy IRQs from hitting vector 0x20.
    unsafe {
        cpu::outb(PIC1_DATA, 0xFF);
        cpu::outb(PIC2_DATA, 0xFF);
    }
}

pub fn enable_x2apic() -> Result<u32, &'static str> {
    if !has_x2apic() {
        return Err("CPUID reports no x2APIC");
    }
    // SAFETY: we are in kernel mode and have already loaded our IDT.
    unsafe {
        let mut base = cpu::rdmsr(IA32_APIC_BASE);
        base |= APIC_BASE_ENABLE | APIC_BASE_X2APIC;
        cpu::wrmsr(IA32_APIC_BASE, base);
        cpu::wrmsr(MSR_SVR, SPURIOUS_VECTOR as u64 | SVR_APIC_ENABLE);
        cpu::wrmsr(MSR_LVT_TIMER, LVT_MASK | TIMER_VECTOR as u64);
    }
    Ok(id())
}

pub fn id() -> u32 {
    // SAFETY: x2APIC is enabled, so MSR 0x802 is the APIC ID.
    unsafe { cpu::rdmsr(MSR_APIC_ID) as u32 }
}

pub fn eoi() {
    // SAFETY: x2APIC EOI is a write of 0 to MSR 0x80B.
    unsafe { cpu::wrmsr(MSR_EOI, 0) }
}

pub fn start_timer(initial_count: u32) {
    // SAFETY: LVT/timer MSRs are valid after enable_x2apic().
    unsafe {
        cpu::wrmsr(MSR_TIMER_DIV, TIMER_DIVIDE_BY_1);
        cpu::wrmsr(
            MSR_LVT_TIMER,
            TIMER_VECTOR as u64 | LVT_PERIODIC,
        );
        cpu::wrmsr(MSR_TIMER_INIT, initial_count as u64);
    }
}

pub fn mask_timer() {
    unsafe {
        cpu::wrmsr(MSR_LVT_TIMER, LVT_MASK | TIMER_VECTOR as u64);
    }
}

pub fn send_ipi_self() {
    unsafe {
        cpu::wrmsr(MSR_ICR, IPI_VECTOR as u64 | ICR_DEST_SELF);
    }
}

pub fn send_ipi_all_except_self() {
    unsafe {
        cpu::wrmsr(MSR_ICR, IPI_VECTOR as u64 | ICR_DEST_ALL_EXCL_SELF);
    }
}
