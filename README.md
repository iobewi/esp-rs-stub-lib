# esp-rs-stub-lib

Experimental `no_std` Rust foundations for Espressif RAM-resident flasher stubs.

The initial target is ESP32-P4 (revision 3.x), preserving the existing `esptool-js` / Web Serial host workflow. The C implementation remains the reference. No board access or destructive operations are part of the foundation phase.

Development is tracked in pull requests; `main` starts from this minimal project description.
