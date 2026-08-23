//! Trusted same-ring Courtlet bring-up: cloned CR3, shared ring, cap revoke.
//!
//! Courtlet code still lives in the kernel higher-half (PML4[511] is shared).
//! Each courtlet has its own lower-half stack, boot info, and a shared ring.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::mm::BumpAllocator;
use crate::object::{self, RootCourt};
use crate::paging::AddressSpace;
use crate::println;

const PAGE: u64 = 0x1000;
const BOOT_VA: u64 = 0x0020_0000;
const RING_VA: u64 = 0x0030_0000;
const STACK_VA: u64 = 0x0040_0000;
const STACK_PAGES: u64 = 8;
const STACK_SIZE: u64 = STACK_PAGES * PAGE;

const BOOT_MAGIC: u64 = 0x434B_424F_4F54_3032; // CKBOOT02
const RING_MAGIC: u64 = 0x434B_5249_4E47_3031; // CKRING01
const SLOT: usize = 64;
const CAPACITY: u32 = 4;
const RING_DATA: usize = 128;

const ST_SENT: u64 = 0x0001;
const ST_RECV: u64 = 0x0002;
const ST_DENIED: u64 = 0xE001;
const ST_EMPTY: u64 = 0xE002;
const ST_BAD_MAGIC: u64 = 0xBAD0;

static SAVED_KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

#[repr(C)]
struct CkBootInfo {
    magic: u64,
    cap_id: u64,
    rights: u64,
    ring_virt: u64,
    court_id: u64,
}

#[repr(C)]
struct Ring {
    magic: u64,
    capacity: u32,
    slot_size: u32,
    producer: AtomicU64,
    consumer: AtomicU64,
    revoked: AtomicU64,
}

struct Courtlet {
    #[allow(dead_code)]
    name: &'static str,
    cr3: u64,
    #[allow(dead_code)]
    space: AddressSpace,
    stack_top: u64,
    boot: *mut CkBootInfo,
    entry: extern "C" fn() -> u64,
}

fn ring_from_va<'a>() -> &'a Ring {
    unsafe { &*(RING_VA as *const Ring) }
}

fn boot_from_va<'a>() -> &'a CkBootInfo {
    unsafe { &*(BOOT_VA as *const CkBootInfo) }
}

fn slot_ptr(index: u64) -> *mut u8 {
    (RING_VA + RING_DATA as u64 + index * SLOT as u64) as *mut u8
}

extern "C" fn app_entry() -> u64 {
    let boot = boot_from_va();
    if boot.magic != BOOT_MAGIC {
        return ST_BAD_MAGIC;
    }
    let ring = ring_from_va();
    if ring.magic != RING_MAGIC {
        return ST_BAD_MAGIC;
    }
    if boot.rights & object::RIGHT_SEND == 0 || ring.revoked.load(Ordering::Acquire) != 0 {
        return ST_DENIED;
    }
    let prod = ring.producer.load(Ordering::Relaxed);
    let cons = ring.consumer.load(Ordering::Acquire);
    if prod.wrapping_sub(cons) >= u64::from(ring.capacity) {
        return ST_DENIED;
    }
    let idx = prod % u64::from(ring.capacity);
    let payload = b"pkt";
    unsafe {
        slot_ptr(idx).cast::<u32>().write(payload.len() as u32);
        slot_ptr(idx).add(4).copy_from_nonoverlapping(payload.as_ptr(), payload.len());
    }
    ring.producer.store(prod + 1, Ordering::Release);
    ST_SENT
}

extern "C" fn net_entry() -> u64 {
    let boot = boot_from_va();
    if boot.magic != BOOT_MAGIC {
        return ST_BAD_MAGIC;
    }
    let ring = ring_from_va();
    if ring.magic != RING_MAGIC {
        return ST_BAD_MAGIC;
    }
    if boot.rights & object::RIGHT_RECV == 0 || ring.revoked.load(Ordering::Acquire) != 0 {
        return ST_DENIED;
    }
    let prod = ring.producer.load(Ordering::Acquire);
    let cons = ring.consumer.load(Ordering::Relaxed);
    if cons == prod {
        return ST_EMPTY;
    }
    let idx = cons % u64::from(ring.capacity);
    let len = unsafe { slot_ptr(idx).cast::<u32>().read() as u64 };
    ring.consumer.store(cons + 1, Ordering::Release);
    ST_RECV | (len << 8)
}

extern "C" fn probe_entry() -> u64 {
    0x11
}

unsafe fn enter(cr3: u64, stack_top: u64, entry: extern "C" fn() -> u64) -> u64 {
    let kernel_cr3 = crate::cpu::cr3() & 0x000f_ffff_ffff_f000;
    let new_cr3 = cr3 & 0x000f_ffff_ffff_f000;
    let ret: u64;
    // `call` clobbers rax; pin CR3 and the entry pointer on callee-saved regs.
    unsafe {
        asm!(
            "mov [{saved}], rsp",
            "mov cr3, r12",
            "mov rsp, r13",
            "call r14",
            "mov cr3, r15",
            "mov rsp, [{saved}]",
            lateout("rax") ret,
            saved = sym SAVED_KERNEL_RSP,
            in("r12") new_cr3,
            in("r13") stack_top,
            in("r14") entry,
            in("r15") kernel_cr3,
            out("rcx") _,
            out("rdx") _,
            out("rsi") _,
            out("rdi") _,
            out("r8") _,
            out("r9") _,
            out("r10") _,
            out("r11") _,
        );
    }
    ret
}

fn spawn(
    bump: &mut BumpAllocator,
    name: &'static str,
    entry: extern "C" fn() -> u64,
    cap_id: u64,
    rights: u64,
    court_id: u64,
    ring_phys: u64,
) -> Result<Courtlet, &'static str> {
    let mut space = AddressSpace::clone_kernel(bump)?;
    let (boot_phys, boot_virt) = bump.alloc_pages(1).ok_or("courtlet boot page")?;
    unsafe { boot_virt.write_bytes(0, PAGE as usize) };
    let boot = boot_virt.cast::<CkBootInfo>();
    unsafe {
        boot.write(CkBootInfo {
            magic: BOOT_MAGIC,
            cap_id,
            rights,
            ring_virt: RING_VA,
            court_id,
        });
    }
    let (stack_phys, stack_virt) = bump
        .alloc_pages(STACK_PAGES)
        .ok_or("courtlet stack")?;
    unsafe { stack_virt.write_bytes(0, STACK_SIZE as usize) };
    space.map(bump, BOOT_VA, boot_phys, PAGE)?;
    space.map(bump, RING_VA, ring_phys, PAGE)?;
    space.map(bump, STACK_VA, stack_phys, STACK_SIZE)?;
    let cr3 = space.cr3;
    Ok(Courtlet {
        name,
        cr3,
        space,
        stack_top: STACK_VA + STACK_SIZE,
        boot,
        entry,
    })
}

/// Load two trusted courtlets, send one packet, then revoke and prove deny.
pub fn run_demo(bump: &mut BumpAllocator, root: &mut RootCourt) -> Result<(), &'static str> {
    let (app_court, app_obj) = root.spawn_court("app", "/court/app")?;
    let (net_court, net_obj) = root.spawn_court("net", "/court/net")?;
    let app_cap = root.mint_cap(app_obj, object::RIGHT_SEND)?;
    let net_cap = root.mint_cap(net_obj, object::RIGHT_RECV)?;

    let (ring_phys, ring_virt) = bump.alloc_pages(1).ok_or("ring page")?;
    unsafe { ring_virt.write_bytes(0, PAGE as usize) };
    let ring = ring_virt.cast::<Ring>();
    unsafe {
        ring.write(Ring {
            magic: RING_MAGIC,
            capacity: CAPACITY,
            slot_size: SLOT as u32,
            producer: AtomicU64::new(0),
            consumer: AtomicU64::new(0),
            revoked: AtomicU64::new(0),
        });
    }

    let app = spawn(
        bump,
        "app",
        app_entry,
        app_cap,
        object::RIGHT_SEND,
        app_court,
        ring_phys,
    )?;
    let net = spawn(
        bump,
        "net",
        net_entry,
        net_cap,
        object::RIGHT_RECV,
        net_court,
        ring_phys,
    )?;

    println!(
        "courtlet: app cr3={:#x} net cr3={:#x} kernel_pml4={:#x}",
        app.cr3,
        net.cr3,
        crate::paging::kernel_pml4_phys()
    );
    let probe = unsafe { enter(app.cr3, app.stack_top, probe_entry) };
    println!("courtlet: stacked probe status={probe:#x}");
    if probe != 0x11 {
        return Err("courtlet stacked probe failed");
    }

    let sent = unsafe { enter(app.cr3, app.stack_top, app.entry) };
    println!("courtlet: app send status={sent:#x}");
    if sent != ST_SENT {
        return Err("app courtlet send failed");
    }

    let recvd = unsafe { enter(net.cr3, net.stack_top, net.entry) };
    println!("courtlet: net recv status={recvd:#x}");
    if recvd & 0xFF != ST_RECV {
        return Err("net courtlet recv failed");
    }

    root.revoke_cap(app_cap)?;
    unsafe {
        (*app.boot).rights = 0;
        (*ring).revoked.store(1, Ordering::Release);
    }
    if root.cap_rights(app_cap) != Some(0) {
        return Err("app cap still live after revoke");
    }

    let denied = unsafe { enter(app.cr3, app.stack_top, app.entry) };
    println!("courtlet: app after revoke status={denied:#x}");
    if denied != ST_DENIED {
        return Err("revoked send did not deny");
    }

    println!(
        "courtlet: ns app={} net={} caps={}",
        root.lookup("/court/app"),
        root.lookup("/court/net"),
        root.cap_count()
    );
    Ok(())
}
