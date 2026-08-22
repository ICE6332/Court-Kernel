use core::fmt::{self, Write};
use spin::Mutex;

const COM1: u16 = 0x3F8;
const ISA_DEBUG_EXIT: u16 = 0xF4;

static SERIAL: Mutex<SerialPort> = Mutex::new(SerialPort { port: COM1 });

pub struct SerialPort {
    port: u16,
}

impl SerialPort {
    fn init(&self) {
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

pub fn qemu_exit_success() -> ! {
    unsafe { outb(ISA_DEBUG_EXIT, 0x10) };
    hcf()
}

pub fn qemu_exit_failure() -> ! {
    unsafe { outb(ISA_DEBUG_EXIT, 0x11) };
    hcf()
}

pub fn hcf() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags)
        );
    }
    value
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
