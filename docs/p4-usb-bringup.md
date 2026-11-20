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

The image deliberately stops servicing traffic on any CDC line change or a
transport error. Initial bring-up uses an external reset to establish a
new session. The line-change and rearm model is tested on the host; the
native ROM interrupt dispatch, same-hart exclusion around ROM calls and
pending RX delivery still require this ECO7 trial. The test exchanges no
flash data and does not establish throughput.

## Source reference

The attach sequence follows the [Espressif P4 wrapper](https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/src/usb_otg.c).
The revision and register layout follow ESP-IDF's
[P4 v3 register definitions](https://github.com/espressif/esp-idf/tree/master/components/soc/esp32p4/register/hw_ver3/soc).
