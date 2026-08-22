//! Kernel IDT: exceptions, APIC timer, IPI, spurious.
//!
//! Every vector 0..=255 has its own 16-byte stub that pushes the *real*
//! vector number (see `idt_stubs.s`). A shared `isr_unexpected` that reports
//! 0xFFFF would hide stray IRQs while writing page tables.

use core::arch::{asm, global_asm, naked_asm};
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::apic;
use crate::cpu::{self, KERNEL_CS};
use crate::serial;

pub const TIMER_VECTOR: u8 = 0x20;
pub const IPI_VECTOR: u8 = 0x30;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

const STUB_STRIDE: u64 = 16;

pub static TIMER_TICKS: AtomicU32 = AtomicU32::new(0);
pub static IPI_PONG: AtomicU32 = AtomicU32::new(0);

global_asm!(include_str!("idt_stubs.s"));

unsafe extern "C" {
    fn isr_stub_table();
}

fn stub(vec: u8) -> u64 {
    isr_stub_table as *const () as u64 + vec as u64 * STUB_STRIDE
}

#[repr(C, packed)]
struct TablePtr {
    limit: u16,
    base: u64,
}

#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    flags: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn empty() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            flags: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn gate(offset: u64, ist: u8) -> Self {
        Self {
            offset_low: offset as u16,
            selector: KERNEL_CS,
            ist,
            flags: 0x8E,
            offset_mid: (offset >> 16) as u16,
            offset_high: (offset >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, align(16))]
struct Idt([IdtEntry; 256]);

static mut IDT: Idt = Idt([IdtEntry::empty(); 256]);

/// Fill the IDT on the BSP, then `lidt` on this CPU.
pub fn init() -> Result<(), &'static str> {
    if stub(1).wrapping_sub(stub(0)) != STUB_STRIDE {
        return Err("ISR stub stride is not 16 bytes");
    }
    // SAFETY: interrupts are off; only the BSP fills the table.
    unsafe {
        let idt = &mut *addr_of_mut!(IDT);
        for vec in 0..=255u8 {
            let ist = if vec == 8 { 1 } else { 0 };
            idt.0[vec as usize] = IdtEntry::gate(stub(vec), ist);
        }
    }
    load();
    Ok(())
}

/// `lidt` using the table filled by [`init`].
pub fn load() {
    let idtr = TablePtr {
        limit: (256 * 16 - 1) as u16,
        base: addr_of!(IDT) as u64,
    };
    // SAFETY: `idtr` points at the kernel IDT.
    unsafe {
        asm!("lidt [{idtr}]", idtr = in(reg) &idtr);
    }
}

#[repr(C)]
struct InterruptFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    vector: u64,
    error: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
unsafe extern "C" fn isr_common() {
    naked_asm!(
        "push rax",
        "push rbx",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push rbp",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "cld",
        "mov rdi, rsp",
        "mov rbp, rsp",
        "and rsp, -16",
        "call {dispatch}",
        "mov rsp, rbp",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rbp",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rbx",
        "pop rax",
        "add rsp, 16",
        "iretq",
        dispatch = sym isr_dispatch,
    );
}

extern "C" fn isr_dispatch(frame: &InterruptFrame) {
    match frame.vector {
        v if v == TIMER_VECTOR as u64 => {
            TIMER_TICKS.fetch_add(1, Ordering::Release);
            apic::eoi();
        }
        v if v == IPI_VECTOR as u64 => {
            IPI_PONG.fetch_add(1, Ordering::Release);
            apic::eoi();
        }
        v if v == SPURIOUS_VECTOR as u64 => {}
        _ => {
            serial::print_raw(format_args!(
                "exception vec={} err={:#x} rip={:#x} cr2={:#x}\n",
                frame.vector,
                frame.error,
                frame.rip,
                cpu::cr2()
            ));
            serial::qemu_exit_failure();
        }
    }
}
