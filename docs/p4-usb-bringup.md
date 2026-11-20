# ESP32-P4 v3.2 USB-OTG RAM bring-up

The opt-in image sends the standard `OHAI` greeting and answers the
diagnostic SLIP payload `PING` with `PONG`. It does not implement a flash
command dispatcher. Keep the image separate from the default foundation
and the link-only smoke ELF.

## Build

```sh
cargo build --release --target riscv32imc-unknown-none-elf \
  -p p4-foundation --features usb-bringup --bin p4-usb-bringup
mkdir -p build
python tools/elf2stub.py \
  target/riscv32imc-unknown-none-elf/release/p4-usb-bringup \
  build/p4-usb-bringup.json
```

## On-board acceptance

1. Use the approved v3.2 board on its native USB-OTG download port.
   Confirm the detected chip revision and port before loading the candidate.
   Keep the USB serial session open across upload and diagnostics.
2. In a disposable esptool-js build, replace only its modern P4 stub JSON
   (`src/targets/stub_flasher/esp32p4.json`) with the generated file. Connect
   to the ROM downloader and run `runStub()`; record the `OHAI` result.
   Use the port with the native P4 USB-OTG VID/PID so the standard loader
   can identify the transport without a post-stub register command.
3. On the same esptool-js `Transport`, send two separate `PING` payloads via
   `Transport.write()` and read two `PONG` payloads via `Transport.read()`,
   one reply per request. The transport adds/removes the SLIP delimiters.
   This exercises RX rearming between requests. Keep any timeout or corrupt
   response as a failure, with the serial trace.
4. Reset externally into ROM download mode, reopen the port if required,
   and repeat the upload and both round trips. Record the exact revision,
   USB identity, artifact SHA-256 and results. Loading is into RAM only.

The CDC callback samples RTS and latches a falling edge or failed status
read. Foreground resolves the event via `RxLineState` with hart 0
interrupts excluded through inspection and acknowledgement. Benign changes
preserve the session and buffered RX data. A falling edge, invalid device
or transport error stops the probe until an external reset.

Each TX reply has a 250 ms retry window based on `mcycle` and the ROM's
CPU ticks per microsecond. A zero acceptance leaves interrupts enabled for
a backoff of up to 100 µs before reoffering pending bytes. An indeterminate
result stops TX immediately. This controls retries between ROM calls and
cannot preempt a stalled ROM call. The last TX outcome is available in the
RAM symbol `P4_USB_TX_DIAGNOSTICS` as four 32-bit words: outcome (1 sent,
2 timeout, 3 FIFO error, 4 invalid link), FIFO calls, accepted bytes, and
zero-acceptance calls.

CI validates the ROM symbols and decoded call targets in the linked
candidate, including the DWC handler pointer, and checks linked reads and
writes for the four ESP-IDF P4 v3 MMIO addresses. Native ROM interrupt
dispatch, same-hart exclusion and pending RX delivery still require the
ECO7 trial. The test exchanges no flash data and does not establish throughput.

## Source reference

The attach sequence follows the [Espressif P4 wrapper](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/src/usb_otg.c).
The revision and register layout follow ESP-IDF's
[P4 v3 register definitions](https://github.com/espressif/esp-idf/tree/master/components/soc/esp32p4/register/hw_ver3/soc).
