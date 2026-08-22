#![no_std]
#![no_main]

mod apic;
mod cpu;
mod gdt;
mod idt;
mod limine_abi;
mod mm;
mod object;
mod serial;

use core::sync::atomic::{AtomicU32, Ordering};
use limine_abi::{
    BaseRevision, BootloaderInfoRequest, FirmwareTypeRequest, HhdmRequest, MemmapRequest, MpInfo,
    MpRequest, RequestsEndMarker, RequestsStartMarker, StackSizeRequest, FIRMWARE_UEFI64,
    MEMMAP_USABLE, MP_X2APIC,
};
use object::RootCourt;

use crate::serial::{hcf, qemu_exit_failure, qemu_exit_success};

#[used]
#[unsafe(link_section = ".requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new(3);

#[used]
#[unsafe(link_section = ".requests")]
static STACK_SIZE: StackSizeRequest = StackSizeRequest::new(0x10000);

#[used]
#[unsafe(link_section = ".requests")]
static BOOTLOADER_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static FIRMWARE: FirmwareTypeRequest = FirmwareTypeRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static HHDM: HhdmRequest = HhdmRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MEMMAP: MemmapRequest = MemmapRequest::new();

#[used]
#[unsafe(link_section = ".requests")]
static MP: MpRequest = MpRequest::new(MP_X2APIC);

#[used]
#[unsafe(link_section = ".requests_start_marker")]
static _START_MARKER: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".requests_end_marker")]
static _END_MARKER: RequestsEndMarker = RequestsEndMarker::new();

static AP_READY: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    serial::init();
    println!("Court Kernel Root Court");
    println!("MVP-1 UEFI bring-up");

    if BASE_REVISION.loaded_revision_valid() {
        println!(
            "limine loaded base revision: {}",
            BASE_REVISION.loaded_revision()
        );
    } else {
        println!("limine loaded base revision: (magic word not rewritten)");
    }
    if !BASE_REVISION.is_supported() {
        println!("warn: limine did not zero the requested base revision");
    }

    if STACK_SIZE.response().is_some() {
        println!("stack: 64KiB request accepted");
    }

    if let Some(info) = BOOTLOADER_INFO.response() {
        println!("bootloader: {} {}", info.name(), info.version());
    }

    if let Some(fw) = FIRMWARE.response() {
        let kind = if fw.firmware_type == FIRMWARE_UEFI64 {
            "uefi64"
        } else {
            "other"
        };
        println!("firmware: {kind} ({})", fw.firmware_type);
        if fw.firmware_type != FIRMWARE_UEFI64 {
            println!("error: expected UEFI64 firmware path");
            qemu_exit_failure();
        }
    } else {
        println!("error: no firmware type response");
        qemu_exit_failure();
    }

    let hhdm = match HHDM.response() {
        Some(hhdm) => {
            println!("hhdm offset: {:#x}", hhdm.offset);
            hhdm.offset
        }
        None => {
            println!("error: no HHDM response");
            qemu_exit_failure();
        }
    };

    let mut usable = 0u64;
    let mut regions = 0u64;
    let map = match MEMMAP.response() {
        Some(map) => map,
        None => {
            println!("error: no memory map");
            qemu_exit_failure();
        }
    };
    for entry in map.entries() {
        regions += 1;
        if entry.kind == MEMMAP_USABLE {
            usable = usable.saturating_add(entry.length);
        }
    }
    println!("memory map: {regions} regions, {usable} usable bytes");
    if usable == 0 {
        println!("error: no usable memory");
        qemu_exit_failure();
    }

    let mut bump = match mm::BumpAllocator::from_memmap(hhdm, map.entries()) {
        Some(bump) => bump,
        None => {
            println!("error: no usable region for bump allocator");
            qemu_exit_failure();
        }
    };
    println!(
        "mm: bump phys {:#x}..{:#x} ({} KiB)",
        bump.start_phys(),
        bump.end_phys(),
        bump.remaining() / 1024
    );
    let Some((scratch_phys, scratch_virt)) = bump.alloc_pages(1) else {
        println!("error: bump alloc failed");
        qemu_exit_failure();
    };
    // SAFETY: the page is HHDM-mapped usable memory we just reserved.
    unsafe { scratch_virt.write(0x5A) };
    // SAFETY: same page, still exclusively owned by bring-up.
    if unsafe { scratch_virt.read() } != 0x5A {
        println!("error: bump page not writable at {:#x}", scratch_phys);
        qemu_exit_failure();
    }
    println!("mm: test page phys={scratch_phys:#x} ok");

    gdt::init();
    idt::init();
    apic::mask_legacy_pic();
    match apic::enable_x2apic() {
        Ok(apic_id) => println!("x2apic: enabled id={apic_id:#x}"),
        Err(error) => {
            println!("error: {error}");
            qemu_exit_failure();
        }
    }
    println!("gdt/idt: kernel cs=0x08 tss=0x18");

    let mut root = RootCourt::new();
    if let Err(error) = root.bootstrap() {
        println!("error: object bootstrap failed: {error}");
        qemu_exit_failure();
    }
    println!(
        "root objects: courts={} caps={} ns:/court/root={}",
        root.court_count(),
        root.cap_count(),
        root.lookup("/court/root")
    );

    let mut cpu_count = 1u32;
    let mut expected_aps = 0u32;
    if let Some(mp) = MP.response() {
        cpu_count = mp.cpus().len() as u32;
        expected_aps = cpu_count.saturating_sub(1);
        println!(
            "mp: {} cpu(s), bsp lapic {}, flags {:#x}",
            cpu_count, mp.bsp_lapic_id, mp.flags
        );
        for cpu in mp.cpus() {
            if cpu.lapic_id != mp.bsp_lapic_id {
                cpu.start(ap_entry);
            }
        }
        let mut spins = 0u32;
        while AP_READY.load(Ordering::Acquire) < expected_aps && spins < 8_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        let ready = AP_READY.load(Ordering::Acquire);
        println!("ap ready: {ready}/{expected_aps}");
        if ready != expected_aps {
            println!("error: AP bring-up timed out");
            qemu_exit_failure();
        }
    } else {
        println!("mp: single cpu (no MP response)");
    }

    apic::start_timer(1_000_000);
    // SAFETY: IDT, x2APIC SVR, and timer LVT are live on this CPU.
    unsafe { cpu::sti() };

    apic::send_ipi_self();
    if !wait_atomic(&idt::IPI_PONG, 1) {
        println!("error: self IPI did not land");
        qemu_exit_failure();
    }
    println!("ipi: self pong ok");

    if !wait_atomic(&idt::TIMER_TICKS, 3) {
        println!(
            "error: x2APIC timer did not tick ({})",
            idt::TIMER_TICKS.load(Ordering::Acquire)
        );
        qemu_exit_failure();
    }
    println!(
        "timer: ticks={}",
        idt::TIMER_TICKS.load(Ordering::Acquire)
    );
    apic::mask_timer();

    if expected_aps > 0 {
        apic::send_ipi_all_except_self();
        let want = 1 + expected_aps;
        if !wait_atomic(&idt::IPI_PONG, want) {
            println!(
                "error: AP IPI pong {}/{}",
                idt::IPI_PONG.load(Ordering::Acquire),
                want
            );
            qemu_exit_failure();
        }
    }
    println!(
        "ipi pong: {} (ICR)",
        idt::IPI_PONG.load(Ordering::Acquire)
    );

    cpu::cli();
    println!("BOOT_OK cpus={cpu_count}");
    qemu_exit_success();
}

fn wait_atomic(cell: &AtomicU32, want: u32) -> bool {
    let start = cpu::rdtsc();
    while cell.load(Ordering::Acquire) < want {
        if cpu::rdtsc().wrapping_sub(start) > 8_000_000_000 {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

extern "C" fn ap_entry(_info: &'static MpInfo) -> ! {
    gdt::load_ap();
    idt::load();
    if apic::enable_x2apic().is_err() {
        hcf();
    }
    // SAFETY: this AP has loaded the kernel IDT and enabled x2APIC.
    unsafe { cpu::sti() };
    AP_READY.fetch_add(1, Ordering::Release);
    loop {
        cpu::hlt();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {info}");
    qemu_exit_failure();
}
