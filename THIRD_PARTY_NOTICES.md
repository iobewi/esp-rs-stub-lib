# Third-party notices

## Espressif — esp-flasher-stub

The Rust SLIP codec and receive double-buffer implementation in
`crates/stub-core/src/slip.rs` and `crates/stub-core/src/rx.rs` are
adapted from the following Espressif C sources:

- https://github.com/espressif/esp-flasher-stub/blob/master/src/slip.c
- https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.c
- https://github.com/espressif/esp-flasher-stub/blob/master/src/frame_buffer.h

The C sources declare `SPDX-License-Identifier: Apache-2.0 OR MIT`.
This project uses the **MIT** license alternative for these adaptations.
The upstream copyright notices are:

- `SPDX-FileCopyrightText: 2025-2026 Espressif Systems (Shanghai) CO LTD`
  (`src/slip.c`)
- `SPDX-FileCopyrightText: 2026 Espressif Systems (Shanghai) CO LTD`
  (`src/frame_buffer.c` and `src/frame_buffer.h`)

The upstream repository distributes the following MIT permission notice in
[`LICENSE-MIT`](https://github.com/espressif/esp-flasher-stub/blob/master/LICENSE-MIT):

MIT License

Copyright (c) 2025 Espressif Systems (Shanghai) CO LTD

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


## Espressif — esp-stub-lib

The P4 USB-OTG attach sequence and ROM download channel check in
`targets/esp32p4/src/bin/p4-usb-bringup.rs` are adapted from:

- https://github.com/espressif/esp-stub-lib/blob/master/src/target/esp32p4/src/usb_otg.c
- https://github.com/espressif/esp-stub-lib/blob/master/src/usb_otg.c

Both source files carry `SPDX-FileCopyrightText: 2025-2026 Espressif Systems
(Shanghai) CO LTD` and `SPDX-License-Identifier: Apache-2.0 OR MIT`.
This adaptation uses the MIT option. The text of esp-stub-lib's
[LICENSE-MIT](https://github.com/espressif/esp-stub-lib/blob/master/LICENSE-MIT)
is identical to the Espressif copyright and permission notice above.
