use core::fmt::{self, Write};
use spin::Mutex;

use crate::cpu::{self, inb, outb};

const COM1: u16 = 0x3F8;
const ISA_DEBUG_EXIT: u16 = 0xF4;

static SERIAL: Mutex<SerialPort> = Mutex::new(SerialPort { port: COM1 });

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    fn init(&self) {
        // SAFETY: COM1 is dedicated to kernel bring-up logs.
        unsafe {
            outb(self.port + 1, 0x00);
            outb(self.port + 3, 0x80);
            outb(self.port, 0x03);
            outb(self.port + 1, 0x00);
            outb(self.port + 3, 0x03);
            outb(self.port + 2, 0xC7);
            outb(self.port + 4, 0x0B);
        }
    }

    fn write_byte(&self, byte: u8) {
        // SAFETY: COM1 transmitter holding register.
        unsafe {
            while inb(self.port + 5) & 0x20 == 0 {
                core::hint::spin_loop();
            }
            outb(self.port, byte);
        }
    }
}

impl Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}

pub fn init() {
    SERIAL.lock().init();
}

pub fn write_fmt(args: fmt::Arguments<'_>) {
    let _ = SERIAL.lock().write_fmt(args);
}

/// Interrupt-safe serial write; may interleave with the locked printer.
pub fn print_raw(args: fmt::Arguments<'_>) {
    let _ = RawSerial.write_fmt(args);
}

struct RawSerial;

impl Write for RawSerial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let port = SerialPort { port: COM1 };
        for byte in s.bytes() {
            if byte == b'\n' {
                port.write_byte(b'\r');
            }
            port.write_byte(byte);
        }
        Ok(())
    }
}

pub fn qemu_exit_success() -> ! {
    // SAFETY: isa-debug-exit is the QEMU bring-up contract (0x10 = success).
    unsafe { outb(ISA_DEBUG_EXIT, 0x10) };
    hcf()
}

pub fn qemu_exit_failure() -> ! {
    // SAFETY: isa-debug-exit is the QEMU bring-up contract (0x11 = failure).
    unsafe { outb(ISA_DEBUG_EXIT, 0x11) };
    hcf()
}

pub fn hcf() -> ! {
    cpu::cli();
    loop {
        cpu::hlt();
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::serial::write_fmt(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => { $crate::print!("{}\n", format_args!($($arg)*)) };
}
