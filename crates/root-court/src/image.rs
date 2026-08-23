//! Minimal ELF64 PT_LOAD mapper for trusted Court Images.
//!
//! Root Court owns loading. Images are independent ELFs, not kernel functions.
//! This parser is intentionally small: 64-bit little-endian x86_64 ET_EXEC,
//! no interpreters, no relocations, lower-half only.

use crate::mm::BumpAllocator;
use crate::paging::AddressSpace;

const PAGE: u64 = 0x1000;
const MAX_PHDRS: usize = 8;
const MAX_SEGMENT: u64 = 128 * 1024;
const ELF_MAGIC: [u8; 4] = [0x7f, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ELFOSABI_NONE: u8 = 0;
const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const EHSIZE: usize = 64;
const PHDR_SIZE: usize = 56;

pub fn load(
    bump: &mut BumpAllocator,
    space: &mut AddressSpace,
    image: &[u8],
) -> Result<u64, &'static str> {
    if image.len() < EHSIZE {
        return Err("court image truncated");
    }
    if image[0..4] != ELF_MAGIC {
        return Err("court image is not ELF");
    }
    if image[4] != ELFCLASS64 || image[5] != ELFDATA2LSB {
        return Err("court image is not ELF64 LE");
    }
    if image[7] != ELFOSABI_NONE && image[7] != 3 {
        return Err("court image ABI");
    }
    let e_type = read_u16(image, 16)?;
    if e_type != ET_EXEC && e_type != 3 {
        return Err("court image is not ET_EXEC/ET_DYN");
    }
    if read_u16(image, 18)? != EM_X86_64 {
        return Err("court image is not x86_64");
    }
    let entry = read_u64(image, 24)?;
    let phoff = read_u64(image, 32)? as usize;
    let phentsize = read_u16(image, 54)? as usize;
    let phnum = read_u16(image, 56)? as usize;
    if phentsize != PHDR_SIZE {
        return Err("court image phentsize");
    }
    if phnum == 0 || phnum > MAX_PHDRS {
        return Err("court image phnum");
    }
    let phdrs_end = phoff
        .checked_add(
            phnum
                .checked_mul(phentsize)
                .ok_or("court image phdr overflow")?,
        )
        .ok_or("court image phdr overflow")?;
    if phdrs_end > image.len() {
        return Err("court image phdrs truncated");
    }

    let mut mapped_entry = false;
    for index in 0..phnum {
        let off = phoff + index * phentsize;
        let p_type = read_u32(image, off)?;
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = read_u64(image, off + 8)?;
        let p_vaddr = read_u64(image, off + 16)?;
        let p_filesz = read_u64(image, off + 32)?;
        let p_memsz = read_u64(image, off + 40)?;
        if p_memsz == 0 {
            continue;
        }
        if p_vaddr >= 0x0000_8000_0000_0000 {
            return Err("court image PT_LOAD is not lower-half");
        }
        if p_memsz > MAX_SEGMENT {
            return Err("court image segment too large");
        }
        if p_filesz > p_memsz {
            return Err("court image filesz > memsz");
        }
        let page_off = p_vaddr & (PAGE - 1);
        let map_va = p_vaddr - page_off;
        let map_len = page_off
            .checked_add(p_memsz)
            .ok_or("court image segment overflow")?;
        let pages = map_len.div_ceil(PAGE);
        let (phys, virt) = bump.alloc_pages(pages).ok_or("court image pages")?;
        unsafe { virt.write_bytes(0, (pages * PAGE) as usize) };
        if p_filesz > 0 {
            let file_start = p_offset as usize;
            let file_end = file_start
                .checked_add(p_filesz as usize)
                .ok_or("court image file overflow")?;
            if file_end > image.len() {
                return Err("court image file truncated");
            }
            unsafe {
                virt.add(page_off as usize).copy_from_nonoverlapping(
                    image[file_start..file_end].as_ptr(),
                    p_filesz as usize,
                );
            }
        }
        space.map(bump, map_va, phys, pages * PAGE)?;
        if entry >= p_vaddr && entry < p_vaddr + p_memsz {
            mapped_entry = true;
        }
    }
    if !mapped_entry {
        return Err("court image entry is not in a PT_LOAD");
    }
    Ok(entry)
}

fn read_u16(buf: &[u8], off: usize) -> Result<u16, &'static str> {
    let bytes = buf.get(off..off + 2).ok_or("court image oob")?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, &'static str> {
    let bytes = buf.get(off..off + 4).ok_or("court image oob")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn read_u64(buf: &[u8], off: usize) -> Result<u64, &'static str> {
    let bytes = buf.get(off..off + 8).ok_or("court image oob")?;
    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}
