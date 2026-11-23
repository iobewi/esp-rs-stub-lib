// SPDX-FileCopyrightText: 2026 Espressif Systems (Shanghai) CO LTD
// SPDX-FileCopyrightText: 2026 IOBEWI
// SPDX-License-Identifier: MIT
//! P4 ECO7, single-SPI NOR. The four-byte opcodes avoid a global address-mode switch.
use core::ptr::{null, null_mut, read_volatile, write_volatile};

unsafe extern "C" {
    fn esp_rom_spiflash_read(address: u32, out: *mut u32, len: i32) -> i32;
    fn spi_flash_attach(pins: u32, legacy: bool);
    fn spi_flash_boot_attach(pins: u32, legacy: bool, boot: bool);
    fn esp_rom_opiflash_exec_cmd_eco7(
        spi: i32,
        mode: i32,
        cmd: u32,
        cmd_bits: i32,
        address: u32,
        address_bits: i32,
        dummy: i32,
        tx: *const u8,
        tx_bits: i32,
        rx: *mut u8,
        rx_bits: i32,
        cs: u32,
        write: bool,
    );
    fn esp_rom_spiflash_unlock() -> i32;
    fn esp_rom_spiflash_config_param(
        id: u32,
        size: u32,
        block: u32,
        sector: u32,
        page: u32,
        mask: u32,
    ) -> i32;
}
pub const SECTOR: u32 = 4096;
pub struct Nor {
    pub capacity: u32,
    ticks: u32,
}

fn command(op: u8, address: Option<u32>, dummy: i32, tx: &[u8], rx: &mut [u8], write: bool) {
    #[repr(align(4))]
    struct Fifo([u8; 32]);
    let mut input = Fifo([0; 32]);
    let mut output = Fifo([0; 32]);
    input.0[..tx.len()].copy_from_slice(tx);
    unsafe {
        esp_rom_opiflash_exec_cmd_eco7(
            1,
            if op == 0x21 { 5 } else { 4 },
            op.into(),
            8,
            address.unwrap_or(0),
            if address.is_some() { 32 } else { 0 },
            dummy,
            if tx.is_empty() {
                null()
            } else {
                input.0.as_ptr()
            },
            (tx.len() * 8) as i32,
            if rx.is_empty() {
                null_mut()
            } else {
                output.0.as_mut_ptr()
            },
            (rx.len() * 8) as i32,
            1,
            write,
        )
    }
    rx.copy_from_slice(&output.0[..rx.len()]);
}

impl Nor {
    pub fn attach(ticks: u32) -> Result<Self, u8> {
        unsafe {
            if read_volatile(0x5012d034 as *const u32) & (1 << 16) != 0
                && read_volatile(0x501153fc as *const u32) & 3 == 3
            {
                spi_flash_boot_attach(0, false, true)
            } else {
                spi_flash_attach(0, false)
            }
            let user = 0x5008d018 as *mut u32;
            write_volatile(user, read_volatile(user) | (1 << 31));
        }
        let mut jedec = [0; 3];
        command(0x9f, None, 0, &[], &mut jedec, false);
        // Qualified board geometry and four-byte instruction set.
        if jedec != [0xc8, 0x40, 0x19] {
            return Err(0xc4);
        }
        Ok(Self {
            capacity: 1 << 25,
            ticks,
        })
    }
    pub fn configure(&self, p: [u32; 6]) -> Result<(), u8> {
        if p[1] != self.capacity || p[2] != 65536 || p[3] != SECTOR || p[4] != 256 {
            return Err(0xc0);
        }
        if unsafe { esp_rom_spiflash_config_param(p[0], p[1], p[2], p[3], p[4], p[5]) } != 0 {
            return Err(0xc4);
        }
        Ok(())
    }
    pub fn bounds(&self, addr: u32, len: u32) -> Result<(), u8> {
        if addr > self.capacity || len > self.capacity - addr {
            Err(0xc0)
        } else {
            Ok(())
        }
    }
    fn ready(&self) -> Result<(), u8> {
        let start = super::cycle_count();
        loop {
            let mut status = [0];
            command(5, None, 0, &[], &mut status, false);
            if status[0] & 1 == 0 {
                return Ok(());
            }
            if super::cycle_count().wrapping_sub(start) >= self.ticks * 2_000_000 {
                return Err(0xc4);
            }
        }
    }
    fn enable(&self) -> Result<(), u8> {
        self.ready()?;
        command(6, None, 0, &[], &mut [], false);
        let mut status = [0];
        command(5, None, 0, &[], &mut status, false);
        if status[0] & 2 == 0 {
            Err(0xc5)
        } else {
            Ok(())
        }
    }
    pub fn read(&self, mut addr: u32, out: &mut [u8]) -> Result<(), u8> {
        self.bounds(addr, out.len() as u32)?;
        self.ready()?;
        // Use the bulk ROM read below the 24-bit boundary. Never let a ROM
        // call straddle that boundary and wrap an address back to zero.
        let bulk = out.len().min(0x1000000u32.saturating_sub(addr) as usize);
        let bulk = if (addr as usize | out.as_ptr() as usize) & 3 == 0 {
            bulk & !3
        } else {
            0
        };
        if bulk != 0 {
            if unsafe { esp_rom_spiflash_read(addr, out.as_mut_ptr().cast(), bulk as i32) } != 0 {
                return Err(0xc4);
            }
            addr += bulk as u32;
        }
        for chunk in out[bulk..].chunks_mut(16) {
            command(0x0c, Some(addr), 8, &[], chunk, false);
            addr += chunk.len() as u32;
        }
        Ok(())
    }
    pub fn erase(&self, addr: u32, len: u32) -> Result<(), u8> {
        self.bounds(addr, len)?;
        if (addr | len) % SECTOR != 0 {
            return Err(0xc0);
        }
        if unsafe { esp_rom_spiflash_unlock() } != 0 {
            return Err(0xc5);
        }
        for off in (addr..addr + len).step_by(SECTOR as usize) {
            self.enable()?;
            command(0x21, Some(off), 0, &[], &mut [], true);
            self.ready()?;
        }
        Ok(())
    }
    pub fn program(&self, mut addr: u32, mut data: &[u8]) -> Result<(), u8> {
        self.bounds(addr, data.len() as u32)?;
        while !data.is_empty() {
            let count = data.len().min(32).min(256 - (addr as usize & 255));
            self.enable()?;
            command(0x12, Some(addr), 0, &data[..count], &mut [], true);
            self.ready()?;
            addr += count as u32;
            data = &data[count..];
        }
        Ok(())
    }
}
