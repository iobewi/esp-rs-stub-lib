/* ROM symbols for ESP32-P4 ECO5+ table; implementation-specific aliases.
 * These are linker constants, not addresses of application-owned code.
 * PR1 doesn't call them.
 */
PROVIDE(esp_rom_cdc_acm_fifo_fill = 0x4FC008D0);
PROVIDE(esp_rom_cdc_acm_fifo_read = 0x4FC008D8);
PROVIDE(esp_rom_cdc_acm_rx_fifo_cnt = 0x4FC008D4);
