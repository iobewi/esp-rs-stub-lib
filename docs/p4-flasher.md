# Integrated P4 RAM flasher candidate

Target: ESP32-P4 v3.2, hart 0, ROM USB-OTG CDC download session, single-SPI
NOR with JEDEC C8 40 19 (32 MiB). The flasher runs in RAM. Flash operations
take effect on the attached NOR. The previously qualified PING/PONG image
remains a separate artifact.

## Build

```sh
cargo build --release --target riscv32imc-unknown-none-elf -p p4-foundation --features flasher --bin p4-flasher-stub
python tools/elf2stub.py target/riscv32imc-unknown-none-elf/release/p4-flasher-stub build/p4-flasher-stub.json
```

Create the build directory first with `mkdir -p build`. The foundation workflow exports the ELF,
JSON and digest. Use the resulting artifact SHA for the hardware run.

## Protocol coverage

| Command | Behaviour |
| --- | --- |
| SYNC | Eight responses with zero header value |
| READ_FLASH | Range checked, up to 4096 bytes per SLIP packet, one packet in flight, exact cumulative ACK, final raw 16-byte MD5 |
| SPI_FLASH_MD5 | ROM MD5 over the requested bytes, 16-byte binary result |
| SPI_ATTACH, SPI_SET_PARAMS | Integrated flash pins, ECO7 rail handling, checked chip geometry |
| ERASE_REGION, ERASE_FLASH | Sector erase, alignment and range checked |
| FLASH_BEGIN/DATA/END | Sequential blocks, XOR and length checks, sector erasure on demand, writes split at page boundaries |
| FLASH_DEFL_BEGIN/DATA/END | ROM tinfl, persistent 32 KiB history, declared output size and end-of-stream checked |
| MEM_BEGIN/DATA/END | Download area 0x4ff55000 through 0x4ff5ffff. Optional entry must be aligned and inside the completed transfer |
| READ_REG, WRITE_REG | SPI1 registers and selected read-only eFuse identification words |
| GET_SECURITY_INFO | 20-byte ROM response |
| CHANGE_BAUDRATE, RUN_USER_CODE, encrypted writes, NAND/plugin commands | Unsupported on this candidate. Use external reset for recovery. An encrypted begin or a request to reboot at flash end is rejected |

WRITE_REG is for trusted esptool clients. Register access to SPI1 can issue raw SPI
commands. The operator's permission to execute this firmware must encompass its
flash programming capability. The earlier PING/PONG allowlist applies only to
the probe.

Validated data blocks are acknowledged before programming. A processing failure
is latched and returned to the next command. A transfer ends successfully only
after its declared blocks and bytes have been consumed. A lost link or invalid
read ACK terminates the read session. Reconnect through the ROM after recovery.

## Compatibility and qualification

Reference sources: esp-flasher-stub at `232a1e3a`, esp-stub-lib at `91e85dc3`
and esptool-js 0.7.0. The host package's readFlash() returns after sending its
last cumulative ACK. It leaves the trailing digest frame for the transport.
Include an explicit SPI_FLASH_MD5 comparison in acceptance. Host tests use
the reference tinfl implementation and a simulated NOR. The published esptool-js
0.7.0 client drives the production packet dispatcher through a native pipe,
programs an 88,003-byte fixture in raw and compressed form, reads it back
in multiple packets, and compares both device digests to the host digest.
The pipe carries already-decoded SLIP frames.

1. Load the identified candidate via the existing RAM loader. Read an agreed
   region twice. Compare both byte streams, their host MD5 and SPI_FLASH_MD5.
   Include an unaligned transfer, a short last packet and a region spanning
   0x01000000. Record elapsed time and useful KiB/s on the same range as the
   historical 45.4 KiB/s measurement.
2. With separate authorization and a sacrificial sector range, erase, write
   raw and compressed images, read back, compare bytes and digests, and check
   adjacent sectors against the pre-run capture.

The integrated image requires board qualification for ROM calls, flash timing,
USB backpressure and throughput.
