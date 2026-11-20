# Miri coverage

GitHub Actions runs Miri separately from the stable ESP32-P4 foundation job.

- `cargo +nightly miri test -p stub-core -p stub-rom` executes the
  host-side Rust unit/integration test suites under the interpreter.
- `targets/esp32p4`, `p4-foundation` and the dangerous
  `p4-rom-link-smoke` target are **not** executed under Miri.
- ROM FFI symbols are only declarations; no hardware initialization or
  ROM call takes place in the current host tests.
- A green Miri job detects supported classes of Rust undefined behavior
  along exercised test paths, but cannot validate P4 ECO7 ROM ABI,
  interrupt concurrency, physical USB performance or unexecuted paths.

## Cargo argument ordering

The Python CI command builder implements the same ordering requirement
raised in [esp-rs/esp-hal#6856](https://github.com/esp-rs/esp-hal/pull/6856):
`subcommand("miri").subcommand_arg("test")` emits adjacent words *before*
`.features()`; test harness options follow `--`. Builder tests cover
multiple subcommand parts, feature ordering, empty features, and argv
generation. We use a small Python helper because this repository has no
Rust `xtask` or existing `CargoArgsBuilder`; we do not copy the upstream
builder or create another Rust workspace package just to launch Miri.

Nightly Miri is isolated in its own job and does not change the stable
RV32IMC linker/toolchain configuration used for firmware builds.
