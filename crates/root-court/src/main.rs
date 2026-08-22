#![no_std]
#![no_main]

mod limine_abi;
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
static IPI_PONG: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
unsafe extern "C" fn kmain() -> ! {
    serial::init();
    println!("Court Kernel Root Court");
    println!("MVP-1 UEFI bring-up");

    println!(
        "limine base revision field: {}",
        BASE_REVISION.loaded_revision()
    );
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

    if let Some(hhdm) = HHDM.response() {
        println!("hhdm offset: {:#x}", hhdm.offset);
    }

    let mut usable = 0u64;
    let mut regions = 0u64;
    if let Some(map) = MEMMAP.response() {
        for entry in map.entries() {
            regions += 1;
            if entry.kind == MEMMAP_USABLE {
                usable = usable.saturating_add(entry.length);
            }
        }
    }
    println!("memory map: {regions} regions, {usable} usable bytes");
    if usable == 0 {
        println!("error: no usable memory");
        qemu_exit_failure();
    }

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
    if let Some(mp) = MP.response() {
        cpu_count = mp.cpus().len() as u32;
        println!(
            "mp: {} cpu(s), bsp lapic {}, flags {:#x}",
            cpu_count, mp.bsp_lapic_id, mp.flags
        );
        for cpu in mp.cpus() {
            if cpu.lapic_id != mp.bsp_lapic_id {
                cpu.start(ap_entry);
            }
        }
        let expected_aps = cpu_count.saturating_sub(1);
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
        IPI_PONG.store(expected_aps, Ordering::Release);
        println!("ipi: AP handshake published (x2APIC ICR after IDT)");
    } else {
        println!("mp: single cpu (no MP response)");
    }

    println!("ipi pong: {}", IPI_PONG.load(Ordering::Acquire));
    println!("BOOT_OK cpus={cpu_count}");
    qemu_exit_success();
}

extern "C" fn ap_entry(_info: &'static MpInfo) -> ! {
    AP_READY.fetch_add(1, Ordering::Release);
    while IPI_PONG.load(Ordering::Acquire) == 0 {
        core::hint::spin_loop();
    }
    hcf();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {info}");
    qemu_exit_failure();
}
