/* ROM symbols for ESP32-P4 ECO5+ table; implementation-specific aliases.
 * These are linker constants, not addresses of application-owned code.
 * PR1 doesn't call them.
 */
PROVIDE(esp_rom_cdc_acm_fifo_fill = 0x4FC008D0);
PROVIDE(esp_rom_cdc_acm_fifo_read = 0x4FC008D8);
PROVIDE(esp_rom_cdc_acm_rx_fifo_cnt = 0x4FC008D4);
/* ROM-owned data slot: content is a pointer to the active CDC ACM object.
 * P4 ECO5+ table, declared compatible with later ECO revisions by Espressif.
 */
PROVIDE(uart_acm_dev = 0x4FFBFFD4);


/* Opt-in P4 v3.2 USB bring-up, from the Espressif ECO5+ ROM table. */
PROVIDE(esp_rom_get_uart_device = 0x4FC000B4);
PROVIDE(esp_rom_esprv_intc_int_set_priority = 0x4FC005AC);
PROVIDE(esp_rom_isr_attach = 0x4FC005D0);
PROVIDE(esp_rom_isr_mask = 0x4FC005D4);
PROVIDE(esp_rom_isr_unmask = 0x4FC005D8);
PROVIDE(esp_rom_cdc_acm_irq_state_enable = 0x4FC008E4);
PROVIDE(esp_rom_cdc_acm_irq_rx_enable = 0x4FC008F0);
PROVIDE(esp_rom_cdc_acm_irq_rx_disable = 0x4FC008F4);
PROVIDE(esp_rom_cdc_acm_irq_callback_set = 0x4FC00900);
PROVIDE(esp_rom_usb_dw_isr_handler = 0x4FC00974);
PROVIDE(esp_rom_get_cpu_ticks_per_us = 0x4FC00040);
PROVIDE(esp_rom_cdc_acm_line_ctrl_get = 0x4FC00908);
PROVIDE(MD5Init = 0x4fc005e0);
PROVIDE(MD5Update = 0x4fc005e4);
PROVIDE(MD5Final = 0x4fc005e8);
PROVIDE(esp_rom_spiflash_read = 0x4fc00158);
PROVIDE(spi_flash_attach = 0x4fc001e8);
PROVIDE(spi_flash_boot_attach = 0x4fc0fe6e);
PROVIDE(esp_rom_opiflash_exec_cmd_eco7 = 0x4fc10b40);
PROVIDE(esp_rom_spiflash_unlock = 0x4fc0015c);
PROVIDE(esp_rom_spiflash_config_param = 0x4fc00168);
PROVIDE(tinfl_decompress = 0x4fc000f8);
PROVIDE(GetSecurityInfoProc = 0x4fc000bc);
PROVIDE(esp_rom_spiflash_erase_chip = 0x4fc0012c);
