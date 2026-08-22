//! Per-CPU kernel GDT + TSS.
//!
//! IDT #DF uses IST1, which the CPU reads from the *current* TR. Sharing one
//! TSS is not enough: the first `ltr` marks that descriptor Busy, and a second
//! CPU `ltr` of the same selector #GPs. Each CPU therefore has its own GDT
//! (selectors still 0x08/0x10/0x18) and its own TSS/IST stack.

use core::arch::asm;
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};

use crate::cpu::{KERNEL_CS, KERNEL_DS, KERNEL_TSS, MAX_CPUS};

const IST_BYTES: usize = 16 * 1024;
const GDT_LEN: usize = 5;

#[derive(Clone, Copy)]
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

const fn empty_tss() -> Tss {
    Tss {
        reserved0: 0,
        rsp0: 0,
        rsp1: 0,
        rsp2: 0,
        reserved1: 0,
        ist: [0; 7],
        reserved2: 0,
        reserved3: 0,
        iomap_base: 0,
    }
}

#[repr(C, packed)]
struct TablePtr {
    limit: u16,
    base: u64,
}

#[derive(Clone, Copy)]
#[repr(align(16))]
struct IstStack(#[allow(dead_code)] [u8; IST_BYTES]);

#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Gdt([u64; GDT_LEN]);

static mut GDTS: [Gdt; MAX_CPUS] = [Gdt([0; GDT_LEN]); MAX_CPUS];
static mut TSSS: [Tss; MAX_CPUS] = [empty_tss(); MAX_CPUS];
static mut ISTS: [IstStack; MAX_CPUS] = [IstStack([0; IST_BYTES]); MAX_CPUS];

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

/// Fill every CPU's GDT/TSS/IST. Does not `lgdt`/`ltr`.
pub fn init() {
    // SAFETY: interrupts are off; only the BSP writes these statics.
    unsafe {
        let gdts = addr_of_mut!(GDTS).cast::<Gdt>();
        let tsss = addr_of_mut!(TSSS).cast::<Tss>();
        let ists = addr_of_mut!(ISTS).cast::<IstStack>();
        for cpu in 0..MAX_CPUS {
            let ist_top = ists.add(cpu) as u64 + IST_BYTES as u64;
            let tss = tsss.add(cpu);
            tss.write(Tss {
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
            let (tss_lo, tss_hi) =
                tss_descriptor(tss as u64, size_of::<Tss>() as u64 - 1);
            gdts.add(cpu).write(Gdt([0, KERNEL_CODE, KERNEL_DATA, tss_lo, tss_hi]));
        }
    }
}

/// `lgdt`, reload segments, and `ltr` this CPU's TSS (selector 0x18).
pub fn load(cpu_id: usize) -> Result<(), &'static str> {
    if cpu_id >= MAX_CPUS {
        return Err("cpu id exceeds per-CPU GDT slots");
    }
    let gdtr = TablePtr {
        limit: (GDT_LEN * 8 - 1) as u16,
        base: unsafe { addr_of!(GDTS).cast::<Gdt>().add(cpu_id) as u64 },
    };
    // SAFETY: `gdtr` points at this CPU's GDT; descriptor 0x18 is its TSS.
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
        asm!(
            "ltr {sel:x}",
            sel = in(reg) KERNEL_TSS,
            options(nostack, preserves_flags)
        );
    }
    Ok(())
}
