//! Kernel IDT: exceptions, APIC timer, IPI, spurious.

use core::arch::asm;
use core::arch::naked_asm;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::apic;
use crate::cpu::{self, KERNEL_CS};
use crate::serial;

pub const TIMER_VECTOR: u8 = 0x20;
pub const IPI_VECTOR: u8 = 0x30;
pub const SPURIOUS_VECTOR: u8 = 0xFF;

pub static TIMER_TICKS: AtomicU32 = AtomicU32::new(0);
pub static IPI_PONG: AtomicU32 = AtomicU32::new(0);

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

    fn gate(handler: unsafe extern "C" fn(), ist: u8) -> Self {
        let offset = handler as usize as u64;
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

macro_rules! isr_no_err {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            naked_asm!(
                "push 0",
                "push {v}",
                "jmp {common}",
                v = const $vec,
                common = sym isr_common,
            );
        }
    };
}

macro_rules! isr_err {
    ($name:ident, $vec:expr) => {
        #[unsafe(naked)]
        unsafe extern "C" fn $name() {
            naked_asm!(
                "push {v}",
                "jmp {common}",
                v = const $vec,
                common = sym isr_common,
            );
        }
    };
}

isr_no_err!(isr_0, 0);
isr_no_err!(isr_1, 1);
isr_no_err!(isr_2, 2);
isr_no_err!(isr_3, 3);
isr_no_err!(isr_4, 4);
isr_no_err!(isr_5, 5);
isr_no_err!(isr_6, 6);
isr_no_err!(isr_7, 7);
isr_err!(isr_8, 8);
isr_no_err!(isr_9, 9);
isr_err!(isr_10, 10);
isr_err!(isr_11, 11);
isr_err!(isr_12, 12);
isr_err!(isr_13, 13);
isr_err!(isr_14, 14);
isr_no_err!(isr_15, 15);
isr_no_err!(isr_16, 16);
isr_err!(isr_17, 17);
isr_no_err!(isr_18, 18);
isr_no_err!(isr_19, 19);
isr_no_err!(isr_20, 20);
isr_err!(isr_21, 21);
isr_no_err!(isr_22, 22);
isr_no_err!(isr_23, 23);
isr_no_err!(isr_24, 24);
isr_no_err!(isr_25, 25);
isr_no_err!(isr_26, 26);
isr_no_err!(isr_27, 27);
isr_no_err!(isr_28, 28);
isr_no_err!(isr_29, 29);
isr_no_err!(isr_30, 30);
isr_no_err!(isr_31, 31);
isr_no_err!(isr_timer, TIMER_VECTOR as u64);
isr_no_err!(isr_ipi, IPI_VECTOR as u64);
isr_no_err!(isr_spurious, SPURIOUS_VECTOR as u64);
isr_no_err!(isr_unexpected, 0xFFFF);

/// Fill the IDT on the BSP, then `lidt` on this CPU.
pub fn init() {
    // SAFETY: interrupts are off; only the BSP fills the table.
    unsafe {
        let idt = &mut *addr_of_mut!(IDT);
        for entry in idt.0.iter_mut() {
            *entry = IdtEntry::gate(isr_unexpected, 0);
        }
        let mut set = |vec: u8, handler: unsafe extern "C" fn(), ist: u8| {
            idt.0[vec as usize] = IdtEntry::gate(handler, ist);
        };
        set(0, isr_0, 0);
        set(1, isr_1, 0);
        set(2, isr_2, 0);
        set(3, isr_3, 0);
        set(4, isr_4, 0);
        set(5, isr_5, 0);
        set(6, isr_6, 0);
        set(7, isr_7, 0);
        set(8, isr_8, 1);
        set(9, isr_9, 0);
        set(10, isr_10, 0);
        set(11, isr_11, 0);
        set(12, isr_12, 0);
        set(13, isr_13, 0);
        set(14, isr_14, 0);
        set(15, isr_15, 0);
        set(16, isr_16, 0);
        set(17, isr_17, 0);
        set(18, isr_18, 0);
        set(19, isr_19, 0);
        set(20, isr_20, 0);
        set(21, isr_21, 0);
        set(22, isr_22, 0);
        set(23, isr_23, 0);
        set(24, isr_24, 0);
        set(25, isr_25, 0);
        set(26, isr_26, 0);
        set(27, isr_27, 0);
        set(28, isr_28, 0);
        set(29, isr_29, 0);
        set(30, isr_30, 0);
        set(31, isr_31, 0);
        set(TIMER_VECTOR, isr_timer, 0);
        set(IPI_VECTOR, isr_ipi, 0);
        set(SPURIOUS_VECTOR, isr_spurious, 0);
    }
    load();
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
