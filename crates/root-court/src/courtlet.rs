//! Trusted Courtlet bring-up: cloned CR3, Court Image load, shared ring, cap revoke.
//!
//! App/net logic lives in independent Court Images (lower-half ELFs). Root Court
//! only maps those images, injects boot info, and enters them. Same-ring is still
//! Trusted Bring-up; this is not VMX and not a syscall ABI.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use court_abi::{
    BOOT_MAGIC, BOOT_VA, BootInfo, CAPACITY, PAGE, RING_MAGIC, RING_VA, Ring, SLOT, ST_DENIED,
    ST_RECV, ST_SENT, STACK_PAGES, STACK_SIZE, STACK_VA,
};

use crate::image;
use crate::mm::BumpAllocator;
use crate::object::{self, RootCourt};
use crate::paging::AddressSpace;
use crate::println;

static SAVED_KERNEL_RSP: AtomicU64 = AtomicU64::new(0);

const APP_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/court-image-app"));
const NET_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/court-image-net"));

struct Courtlet {
    #[allow(dead_code)]
    name: &'static str,
    cr3: u64,
    #[allow(dead_code)]
    space: AddressSpace,
    stack_top: u64,
    boot: *mut BootInfo,
    entry: u64,
}

unsafe fn enter(cr3: u64, stack_top: u64, entry: u64) -> u64 {
    let kernel_cr3 = crate::cpu::cr3() & 0x000f_ffff_ffff_f000;
    let new_cr3 = cr3 & 0x000f_ffff_ffff_f000;
    let ret: u64;
    // `call` clobbers rax; pin CR3 and the image entry on callee-saved regs.
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
    image_bytes: &[u8],
    cap_id: u64,
    rights: u64,
    court_id: u64,
    ring_phys: u64,
) -> Result<Courtlet, &'static str> {
    let mut space = AddressSpace::clone_kernel(bump)?;
    let entry = image::load(bump, &mut space, image_bytes)?;
    let (boot_phys, boot_virt) = bump.alloc_pages(1).ok_or("courtlet boot page")?;
    unsafe { boot_virt.write_bytes(0, PAGE as usize) };
    let boot = boot_virt.cast::<BootInfo>();
    unsafe {
        boot.write(BootInfo {
            magic: BOOT_MAGIC,
            cap_id,
            rights,
            ring_virt: RING_VA,
            court_id,
        });
    }
    let (stack_phys, stack_virt) = bump.alloc_pages(STACK_PAGES).ok_or("courtlet stack")?;
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

/// Load two Court Images, send one packet, then revoke and prove deny.
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
        APP_IMAGE,
        app_cap,
        object::RIGHT_SEND,
        app_court,
        ring_phys,
    )?;
    let net = spawn(
        bump,
        "net",
        NET_IMAGE,
        net_cap,
        object::RIGHT_RECV,
        net_court,
        ring_phys,
    )?;

    println!(
        "courtlet: app cr3={:#x} net cr3={:#x} app_image={:#x} net_image={:#x}",
        app.cr3, net.cr3, app.entry, net.entry
    );
    if app.entry >= 0x0000_8000_0000_0000 || net.entry >= 0x0000_8000_0000_0000 {
        return Err("court image entry is not lower-half");
    }

    let sent = unsafe { enter(app.cr3, app.stack_top, app.entry) };
    println!("courtlet: app send status={sent:#x}");
    if sent != ST_SENT {
        return Err("app court image send failed");
    }

    let recvd = unsafe { enter(net.cr3, net.stack_top, net.entry) };
    println!("courtlet: net recv status={recvd:#x}");
    if recvd & 0xFF != ST_RECV {
        return Err("net court image recv failed");
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
