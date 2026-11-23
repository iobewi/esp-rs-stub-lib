// Native link of the ECO7 NOR implementation against instrumented ROM functions.
#[path = "../../targets/esp32p4/src/flash.rs"]
mod flash;
use std::cell::RefCell;
use std::ptr;
const BOUNDARY: usize = 0x1000000;
struct Sim {
    bytes: Vec<u8>,
    powered: bool,
    normal: u32,
    boot: u32,
    geometry: bool,
    wel: bool,
    bulk: u32,
    read4: u32,
}
thread_local! { static SIM:RefCell<Sim>=RefCell::new(Sim{bytes:vec![0xff;2*BOUNDARY],powered:false,normal:0,boot:0,geometry:false,wel:false,bulk:0,read4:0}); }
fn cycle_count() -> u32 {
    1
}
#[no_mangle]
extern "C" fn spi_flash_attach(_: u32, _: bool) {
    SIM.with_borrow_mut(|s| s.normal += 1)
}
#[no_mangle]
extern "C" fn spi_flash_boot_attach(_: u32, _: bool, boot: bool) {
    assert!(boot);
    SIM.with_borrow_mut(|s| {
        assert!(s.powered);
        s.boot += 1
    })
}
#[no_mangle]
extern "C" fn esp_rom_spiflash_unlock() -> i32 {
    0
}
#[no_mangle]
extern "C" fn esp_rom_spiflash_config_param(
    id: u32,
    n: u32,
    b: u32,
    s: u32,
    p: u32,
    m: u32,
) -> i32 {
    assert_eq!(
        [id, n, b, s, p, m],
        [0xc84019, 1 << 25, 65536, 4096, 256, 0xffff]
    );
    SIM.with_borrow_mut(|s| s.geometry = true);
    0
}
#[no_mangle]
unsafe extern "C" fn esp_rom_spiflash_read(addr: u32, out: *mut u32, len: i32) -> i32 {
    SIM.with_borrow_mut(|s| {
        assert!(s.geometry);
        let a = addr as usize;
        let n = len as usize;
        assert!(a + n <= BOUNDARY);
        assert_eq!((a | n | out as usize) & 3, 0);
        unsafe { ptr::copy_nonoverlapping(s.bytes[a..].as_ptr(), out.cast(), n) };
        s.bulk += 1;
    });
    0
}
#[no_mangle]
extern "C" fn esp_rom_spiflash_erase_chip() -> i32 {
    SIM.with_borrow_mut(|s| s.bytes.fill(0xff));
    0
}
#[no_mangle]
unsafe extern "C" fn esp_rom_opiflash_exec_cmd_eco7(
    spi: i32,
    mode: i32,
    op: u32,
    cmd_bits: i32,
    addr: u32,
    addr_bits: i32,
    dummy: i32,
    tx: *const u8,
    nt: i32,
    rx: *mut u8,
    nr: i32,
    cs: u32,
    write: bool,
) {
    assert_eq!((spi, cmd_bits, cs), (1, 8, 1));
    assert_eq!(mode, if op == 0x21 { 5 } else { 4 });
    SIM.with_borrow_mut(|s| {
        let a = addr as usize;
        unsafe {
            match op {
                0x9f => ptr::copy_nonoverlapping([0xc8, 0x40, 0x19].as_ptr(), rx, 3),
                5 => rx.write(if s.wel { 2 } else { 0 }),
                6 => s.wel = true,
                0x0c => {
                    assert_eq!((addr_bits, dummy, write), (32, 8, false));
                    assert!(nr <= 128);
                    ptr::copy_nonoverlapping(s.bytes[a..].as_ptr(), rx, nr as usize / 8);
                    s.read4 += 1;
                }
                0x12 => {
                    assert!(s.wel && write);
                    s.wel = false;
                    let n = nt as usize / 8;
                    assert!(n <= 32 && a % 256 + n <= 256);
                    for (d, v) in s.bytes[a..a + n]
                        .iter_mut()
                        .zip(std::slice::from_raw_parts(tx, n))
                    {
                        *d &= v;
                    }
                }
                0x21 => {
                    assert!(s.wel && write && a % 4096 == 0);
                    s.wel = false;
                    s.bytes[a..a + 4096].fill(0xff);
                }
                _ => panic!("unexpected SPI opcode {op}"),
            }
        }
    })
}
fn main() {
    unsafe extern "C" {
        fn mmap(at: *mut u8, n: usize, prot: i32, flags: i32, fd: i32, offset: isize) -> *mut u8;
    }
    for addr in [0x5012d000usize, 0x50115000, 0x5008d000] {
        assert_eq!(
            unsafe { mmap(addr as *mut u8, 4096, 3, 0x100022, -1, 0) } as usize,
            addr
        );
    }
    let _ = flash::Nor::attach(100).unwrap();
    unsafe {
        ptr::write_volatile(0x5012d034 as *mut u32, 1 << 16);
        ptr::write_volatile(0x501153fc as *mut u32, 3);
    }
    SIM.with_borrow_mut(|s| s.powered = true);
    let nor = flash::Nor::attach(100).unwrap();
    let addr = BOUNDARY as u32 - 4;
    let data: [u8; 36] = std::array::from_fn(|i| i as u8);
    nor.program(addr, &data).unwrap();
    #[repr(align(4))]
    struct Aligned([u8; 36]);
    let mut read = Aligned([0; 36]);
    nor.read(addr, &mut read.0).unwrap();
    assert_eq!(read.0, data);
    nor.erase(BOUNDARY as u32, 4096).unwrap();
    nor.read(addr, &mut read.0).unwrap();
    assert_eq!(read.0[..4], data[..4]);
    assert_eq!(read.0[4..], [0xff; 32]);
    assert!(nor.read(nor.capacity - 2, &mut read.0).is_err());
    SIM.with_borrow(|s| assert_eq!((s.normal, s.boot, s.bulk, s.read4), (1, 1, 2, 4)));
}
