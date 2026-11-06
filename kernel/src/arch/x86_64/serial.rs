//! Ovladac UART 16550 pro ladici vystup.
//!
//! Jde o docasnou vyjimku z pravidla "ovladace v userspace": kernel potrebuje
//! ladici vystup driv, nez existuje jakykoli userspace.

use core::arch::asm;
use core::fmt;

pub struct SerialPort {
    base: u16,
}

const DATA: u16 = 0;
const INT_ENABLE: u16 = 1;
const FIFO_CTRL: u16 = 2;
const LINE_CTRL: u16 = 3;
const MODEM_CTRL: u16 = 4;
const LINE_STATUS: u16 = 5;

const LINE_STATUS_THR_EMPTY: u8 = 1 << 5;

impl SerialPort {
    /// # Safety
    ///
    /// `base` musi byt I/O port UARTu 16550 (nebo nevyuzity port) a nikdo
    /// jiny ho nesmi soucasne pouzivat k jinemu ucelu.
    pub const unsafe fn new(base: u16) -> Self {
        Self { base }
    }

    /// Nastavi 115200 baud, 8N1, FIFO, bez preruseni.
    pub fn init(&mut self) {
        self.write_reg(INT_ENABLE, 0x00); // bez preruseni
        self.write_reg(LINE_CTRL, 0x80); // DLAB = 1, nastaveni delice
        self.write_reg(DATA, 0x01); // delic 1 = 115200 baud (low byte)
        self.write_reg(INT_ENABLE, 0x00); // (high byte)
        self.write_reg(LINE_CTRL, 0x03); // DLAB = 0, 8 bitu, bez parity, 1 stop bit
        self.write_reg(FIFO_CTRL, 0xC7); // FIFO zapnute, vycistene, prah 14 B
        self.write_reg(MODEM_CTRL, 0x03); // DTR + RTS
    }

    pub fn write_byte(&mut self, byte: u8) {
        // Omezeny pocet pokusu, aby chybejici nebo zaseknuty UART nezastavil kernel.
        for _ in 0..100_000 {
            if self.read_reg(LINE_STATUS) & LINE_STATUS_THR_EMPTY != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        self.write_reg(DATA, byte);
    }

    fn write_reg(&mut self, reg: u16, value: u8) {
        // SAFETY: port patri tomuto UARTu (viz `new`), zapis do registru
        // UARTu neovlivnuje pamet.
        unsafe {
            asm!("out dx, al", in("dx") self.base + reg, in("al") value, options(nomem, nostack, preserves_flags))
        };
    }

    fn read_reg(&self, reg: u16) -> u8 {
        let value: u8;
        // SAFETY: viz `write_reg`.
        unsafe {
            asm!("in al, dx", in("dx") self.base + reg, out("al") value, options(nomem, nostack, preserves_flags))
        };
        value
    }
}

impl fmt::Write for SerialPort {
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
