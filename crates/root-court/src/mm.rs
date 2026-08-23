//! Physical bump allocator over Limine usable memory.
//!
//! Base revision 3 HHDM only maps usable / reclaimable / executable /
//! framebuffer regions. This allocator therefore only hands out `USABLE`
//! pages, which are guaranteed to be in the HHDM.

use crate::limine_abi::{MemmapEntry, MEMMAP_USABLE};

const PAGE: u64 = 0x1000;
const MIN_REGION: u64 = 64 * 1024;

#[derive(Clone, Copy)]
pub struct BumpAllocator {
    hhdm: u64,
    start: u64,
    cursor: u64,
    end: u64,
}

impl BumpAllocator {
    pub fn from_memmap(hhdm: u64, entries: &[&MemmapEntry]) -> Option<Self> {
        let mut best: Option<&MemmapEntry> = None;
        for entry in entries {
            if entry.kind != MEMMAP_USABLE {
                continue;
            }
            if best.is_none_or(|current| entry.length > current.length) {
                best = Some(entry);
            }
        }
        let entry = best?;
        let mut start = entry.base.saturating_add(PAGE - 1) & !(PAGE - 1);
        if start < PAGE {
            start = PAGE;
        }
        let end = entry.base.saturating_add(entry.length);
        if end.saturating_sub(start) < MIN_REGION {
            return None;
        }
        Some(Self {
            hhdm,
            start,
            cursor: start,
            end,
        })
    }

    pub fn start_phys(&self) -> u64 {
        self.start
    }

    pub fn end_phys(&self) -> u64 {
        self.end
    }

    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.cursor)
    }

    pub fn virt(&self, phys: u64) -> *mut u8 {
        (self.hhdm.wrapping_add(phys)) as *mut u8
    }

    pub fn alloc_pages(&mut self, pages: u64) -> Option<(u64, *mut u8)> {
        let size = pages.checked_mul(PAGE)?;
        let cursor = self.cursor.saturating_add(PAGE - 1) & !(PAGE - 1);
        let next = cursor.checked_add(size)?;
        if next > self.end {
            return None;
        }
        self.cursor = next;
        Some((cursor, self.virt(cursor)))
    }
}
