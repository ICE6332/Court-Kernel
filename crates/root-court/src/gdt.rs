//! Kernel GDT + TSS. Replaces the bootloader tables so they can be reclaimed.

use core::arch::asm;
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};

use crate::cpu::{KERNEL_CS, KERNEL_DS, KERNEL_TSS};

const IST_BYTES: usize = 16 * 1024;

#[repr(C, packed)]
struct Tss {
    reserved0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap_base: u16,
}

const _: () = assert!(size_of::<Tss>() == 104);

#[repr(C, packed)]
struct TablePtr {
    limit: u16,
    base: u64,
}

#[repr(align(16))]
struct IstStack([u8; IST_BYTES]);

static mut GDT: [u64; 5] = [0; 5];
static mut TSS: Tss = Tss {
    reserved0: 0,
    rsp0: 0,
    rsp1: 0,
    rsp2: 0,
    reserved1: 0,
    ist: [0; 7],
    reserved2: 0,
    reserved3: 0,
    iomap_base: 0,
};
static mut IST_STACK: IstStack = IstStack([0; IST_BYTES]);

const KERNEL_CODE: u64 = 0x0020_9A00_0000_0000;
const KERNEL_DATA: u64 = 0x0000_9200_0000_0000;

fn tss_descriptor(base: u64, limit: u64) -> (u64, u64) {
    let low = (limit & 0xFFFF)
        | ((base & 0xFF_FFFF) << 16)
        | (0x89u64 << 40)
        | (((limit >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    let high = base >> 32;
    (low, high)
}

/// Fill the GDT/TSS once on the BSP, then load them on this CPU.
pub fn init() {
    // SAFETY: interrupts are off; only the BSP writes these statics.
    unsafe {
        let ist_top = addr_of!(IST_STACK.0) as u64 + IST_BYTES as u64;
        addr_of_mut!(TSS).write(Tss {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [ist_top, 0, 0, 0, 0, 0, 0],
            reserved2: 0,
            reserved3: 0,
            iomap_base: size_of::<Tss>() as u16,
        });
        let (tss_lo, tss_hi) = tss_descriptor(addr_of!(TSS) as u64, size_of::<Tss>() as u64 - 1);
        addr_of_mut!(GDT).write([0, KERNEL_CODE, KERNEL_DATA, tss_lo, tss_hi]);
    }
    load();
}

/// `lgdt`, reload segments, and `ltr`. BSP only: a TSS selector is Busy after
/// the first `ltr`, so APs must not load the same descriptor.
pub fn load() {
    load_gdt();
    // SAFETY: descriptor 0x18 is an available 64-bit TSS filled by init().
    unsafe {
        asm!(
            "ltr {sel:x}",
            sel = in(reg) KERNEL_TSS,
            options(nostack, preserves_flags)
        );
    }
}

/// `lgdt` and reload segments. APs use this so they do not #GP on a Busy TSS.
pub fn load_ap() {
    load_gdt();
}

fn load_gdt() {
    let gdtr = TablePtr {
        limit: (5 * 8 - 1) as u16,
        base: addr_of!(GDT) as u64,
    };
    // SAFETY: `gdtr` points at the kernel GDT; this is the defined load sequence.
    unsafe {
        asm!(
            "lgdt [{gdtr}]",
            "push {code}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov {tmp:r}, {data}",
            "mov ds, {tmp:x}",
            "mov es, {tmp:x}",
            "mov ss, {tmp:x}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            gdtr = in(reg) &gdtr,
            code = const KERNEL_CS as u64,
            data = const KERNEL_DS as u64,
            tmp = out(reg) _,
        );
    }
}
