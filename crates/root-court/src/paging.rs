//! Own 4-level page tables that copy Limine revision-3 mapping semantics.
//!
//! First slice: kernel higher-half + restrictive HHDM, then `mov cr3`.
//! Courtlet address spaces come later.
//!
//! Deferred, do not "just" do these later without the matching prelude:
//! - Do not reclaim bootloader-reclaimable until every CPU has switched off
//!   the Limine stack (those stacks live in reclaimable). Own per-CPU stacks,
//!   then `mov rsp`, then reclaim.
//! - PTE NX (bit 63) requires `IA32_EFER.NXE` first; the reverse order is a
//!   reserved-bit `#PF`. W^X is postponed on purpose.
//! - Independent Courtlet CR3s should clone the shared kernel higher-half
//!   PML4 entry (index 511), not remap the kernel from scratch.

use core::ptr::addr_of;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::cpu;
use crate::limine_abi::{
    hhdm_mapped_rev3, MemmapEntry, MEMMAP_EXECUTABLE_AND_MODULES,
};
use crate::mm::BumpAllocator;

const PAGE: u64 = 0x1000;
const HUGE_2M: u64 = 0x200000;
const PRESENT: u64 = 1 << 0;
const WRITE: u64 = 1 << 1;
const HUGE: u64 = 1 << 7;
const PHYS_MASK: u64 = 0x000f_ffff_ffff_f000;
const TABLE_FLAGS: u64 = PRESENT | WRITE;
// NX is omitted until IA32_EFER.NXE is set; bit 63 is reserved without it.
const PAGE_FLAGS: u64 = PRESENT | WRITE;

static ROOT_CR3: AtomicU64 = AtomicU64::new(0);
static KERNEL_PML4_PHYS: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

pub struct PageMap {
    pub pml4_phys: u64,
    pub kernel_virt: u64,
    pub kernel_phys: u64,
    pub kernel_len: u64,
    pub hhdm_bytes: u64,
    pub hhdm_spans: u64,
    pub table_pages: u64,
}

/// Single-PML4 mapper. Courtlet address spaces need a clone of the shared
/// kernel higher-half PML4 entry (index 511), not a second walk of `map_range`.
struct Mapper<'a> {
    bump: &'a mut BumpAllocator,
    pml4_phys: u64,
    table_pages: u64,
}

impl<'a> Mapper<'a> {
    fn virt(&self, phys: u64) -> *mut u8 {
        self.bump.virt(phys)
    }

    fn alloc_table(&mut self) -> Result<u64, &'static str> {
        let (phys, virt) = self.bump.alloc_pages(1).ok_or("out of page-table pages")?;
        // SAFETY: freshly reserved usable page, exclusive to the mapper.
        unsafe { virt.write_bytes(0, PAGE as usize) };
        self.table_pages += 1;
        Ok(phys)
    }

    fn entry(&self, table_phys: u64, index: usize) -> *mut u64 {
        self.virt(table_phys).cast::<u64>().wrapping_add(index)
    }

    fn ensure_table(&mut self, table_phys: u64, index: usize) -> Result<u64, &'static str> {
        let slot = self.entry(table_phys, index);
        // SAFETY: slot points at a mapper-owned table page via HHDM.
        let current = unsafe { slot.read() };
        if current & PRESENT != 0 {
            if current & HUGE != 0 {
                return Err("huge/4k page collision");
            }
            return Ok(current & PHYS_MASK);
        }
        let next = self.alloc_table()?;
        unsafe { slot.write(next | TABLE_FLAGS) };
        Ok(next)
    }

    fn map_4k(&mut self, virt: u64, phys: u64) -> Result<(), &'static str> {
        let pdpt = self.ensure_table(self.pml4_phys, pml4_index(virt))?;
        let pd = self.ensure_table(pdpt, pdpt_index(virt))?;
        let pt = self.ensure_table(pd, pd_index(virt))?;
        let slot = self.entry(pt, pt_index(virt));
        let current = unsafe { slot.read() };
        let want = (phys & PHYS_MASK) | PAGE_FLAGS;
        if current & PRESENT != 0 {
            if current & PHYS_MASK != phys & PHYS_MASK {
                return Err("4k map conflict");
            }
            return Ok(());
        }
        unsafe { slot.write(want) };
        Ok(())
    }

    fn map_2m(&mut self, virt: u64, phys: u64) -> Result<(), &'static str> {
        let pdpt = self.ensure_table(self.pml4_phys, pml4_index(virt))?;
        let pd = self.ensure_table(pdpt, pdpt_index(virt))?;
        let slot = self.entry(pd, pd_index(virt));
        let current = unsafe { slot.read() };
        let want = (phys & PHYS_MASK) | PAGE_FLAGS | HUGE;
        if current & PRESENT != 0 {
            if current & HUGE == 0 {
                return Err("huge/4k page collision");
            }
            if current & PHYS_MASK != phys & PHYS_MASK {
                return Err("2m map conflict");
            }
            return Ok(());
        }
        unsafe { slot.write(want) };
        Ok(())
    }

    fn map_range(&mut self, mut virt: u64, mut phys: u64, mut len: u64) -> Result<(), &'static str> {
        if virt & (PAGE - 1) != 0 || phys & (PAGE - 1) != 0 {
            return Err("map_range not 4k aligned");
        }
        len &= !(PAGE - 1);
        while len >= PAGE {
            if len >= HUGE_2M && (virt | phys) & (HUGE_2M - 1) == 0 {
                self.map_2m(virt, phys)?;
                virt += HUGE_2M;
                phys += HUGE_2M;
                len -= HUGE_2M;
            } else {
                self.map_4k(virt, phys)?;
                virt += PAGE;
                phys += PAGE;
                len -= PAGE;
            }
        }
        Ok(())
    }
}

fn pml4_index(va: u64) -> usize {
    ((va >> 39) & 0x1ff) as usize
}
fn pdpt_index(va: u64) -> usize {
    ((va >> 30) & 0x1ff) as usize
}
fn pd_index(va: u64) -> usize {
    ((va >> 21) & 0x1ff) as usize
}
fn pt_index(va: u64) -> usize {
    ((va >> 12) & 0x1ff) as usize
}

fn align_up(value: u64, align: u64) -> u64 {
    value.saturating_add(align - 1) & !(align - 1)
}

fn align_down(value: u64, align: u64) -> u64 {
    value & !(align - 1)
}

/// 5-level paging: CR3 is a PML5. Kernel and HHDM both live in PML5[511].
fn wrap_la57(mapper: &mut Mapper<'_>, pml4_phys: u64) -> Result<u64, &'static str> {
    let pml5_phys = mapper.alloc_table()?;
    let slot = mapper.entry(pml5_phys, 511);
    unsafe { slot.write(pml4_phys | TABLE_FLAGS) };
    Ok(pml5_phys)
}

pub fn kernel_virt_bounds() -> (u64, u64) {
    (
        addr_of!(__kernel_start) as u64,
        addr_of!(__kernel_end) as u64,
    )
}

pub fn build(
    bump: &mut BumpAllocator,
    hhdm: u64,
    kernel_phys_base: u64,
    kernel_virt_base: u64,
    entries: &[&MemmapEntry],
) -> Result<PageMap, &'static str> {
    let (kstart, kend) = kernel_virt_bounds();
    if kend <= kstart {
        return Err("kernel image has zero size");
    }
    if kstart < kernel_virt_base {
        return Err("kernel start is below executable virtual_base");
    }
    let kernel_phys = kernel_phys_base + (kstart - kernel_virt_base);
    let kernel_len = align_up(kend - kstart, PAGE);

    let pml4_phys = {
        let mut mapper_alloc = Mapper {
            bump,
            pml4_phys: 0,
            table_pages: 0,
        };
        mapper_alloc.alloc_table()?
    };

    KERNEL_PML4_PHYS.store(pml4_phys, Ordering::Release);

    let mut mapper = Mapper {
        bump,
        pml4_phys,
        table_pages: 1,
    };
    mapper.map_range(kstart, kernel_phys, kernel_len)?;

    let mut hhdm_bytes = 0u64;
    let mut hhdm_spans = 0u64;
    for entry in entries {
        if entry.length == 0 {
            continue;
        }
        let start = align_up(entry.base, PAGE);
        let end = align_down(entry.base.saturating_add(entry.length), PAGE);
        if end <= start {
            continue;
        }
        let len = end - start;
        if entry.kind == MEMMAP_EXECUTABLE_AND_MODULES && start >= kernel_phys_base {
            let virt = kernel_virt_base + (start - kernel_phys_base);
            mapper.map_range(virt, start, len)?;
        }
        if !hhdm_mapped_rev3(entry.kind) {
            continue;
        }
        mapper.map_range(hhdm.wrapping_add(start), start, len)?;
        hhdm_bytes = hhdm_bytes.saturating_add(len);
        hhdm_spans += 1;
    }

    let cr3_phys = if cpu::cr4() & cpu::CR4_LA57 != 0 {
        wrap_la57(&mut mapper, pml4_phys)?
    } else {
        pml4_phys
    };

    Ok(PageMap {
        pml4_phys: cr3_phys,
        kernel_virt: kstart,
        kernel_phys,
        kernel_len,
        hhdm_bytes,
        hhdm_spans,
        table_pages: mapper.table_pages,
    })
}

pub fn publish(pml4_phys: u64) {
    ROOT_CR3.store(pml4_phys, Ordering::Release);
}

pub fn published_cr3() -> u64 {
    ROOT_CR3.load(Ordering::Acquire)
}

pub unsafe fn activate(pml4_phys: u64) {
    publish(pml4_phys);
    // SAFETY: caller built tables that cover RIP, stack, GDT, IDT, and HHDM.
    unsafe { cpu::load_cr3(pml4_phys) };
}

pub fn load_published() -> Result<(), &'static str> {
    let phys = published_cr3();
    if phys == 0 {
        return Err("root CR3 not published");
    }
    unsafe { cpu::load_cr3(phys) };
    Ok(())
}

pub fn kernel_pml4_phys() -> u64 {
    KERNEL_PML4_PHYS.load(Ordering::Acquire)
}

/// Isolated lower-half address space that clones kernel PML4[511].
pub struct AddressSpace {
    pub cr3: u64,
    pml4_phys: u64,
}

impl AddressSpace {
    pub fn clone_kernel(bump: &mut BumpAllocator) -> Result<Self, &'static str> {
        let kernel_pml4 = kernel_pml4_phys();
        if kernel_pml4 == 0 {
            return Err("kernel PML4 not published");
        }
        let mut mapper = Mapper {
            bump,
            pml4_phys: 0,
            table_pages: 0,
        };
        let new_pml4 = mapper.alloc_table()?;
        mapper.pml4_phys = new_pml4;
        // Copy every high (and currently empty low) PML4 slot so HHDM and
        // kernel higher-half both stay reachable after the CR3 switch.
        for index in 0..512 {
            let src = mapper.entry(kernel_pml4, index);
            let dst = mapper.entry(new_pml4, index);
            // SAFETY: both slots are mapper-owned table pages via HHDM.
            unsafe { dst.write(src.read()) };
        }
        let cr3 = if cpu::cr4() & cpu::CR4_LA57 != 0 {
            wrap_la57(&mut mapper, new_pml4)?
        } else {
            new_pml4
        };
        Ok(Self {
            cr3,
            pml4_phys: new_pml4,
        })
    }


    pub fn map(
        &mut self,
        bump: &mut BumpAllocator,
        virt: u64,
        phys: u64,
        len: u64,
    ) -> Result<(), &'static str> {
        if virt >= 0x0000_8000_0000_0000 {
            return Err("courtlet map must be lower-half");
        }
        let mut mapper = Mapper {
            bump,
            pml4_phys: self.pml4_phys,
            table_pages: 0,
        };
        mapper.map_range(virt, phys, len)
    }
}
