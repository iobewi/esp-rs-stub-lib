import unittest

from tools.check_p4_rom_link import (
    ROM_RX_COUNT,
    ROM_RX_READ,
    find_handler_address,
    find_mmio_accesses,
    find_pointer_load,
    find_rom_call,
)

# Exact instruction sequence observed in RV32IMC CI on P4 ECO5+ linker.
SMOKE_DISASSEMBLY = """
4ff5002e <rust_entry>:
4ff5002e:    lui a0, 0x4ffc0
4ff50032:    lw  a0, -0x2c(a0)
4ff50036:    beqz a0, 0x4ff50064 <rust_entry+0x36>
4ff5004c:    auipc ra, 0xffcb1
4ff50050:    jalr -0x77c(ra) <esp_rom_cdc_acm_fifo_fill>
"""


class RomLinkDisassemblyTests(unittest.TestCase):
    def test_real_pointer_load(self):
        self.assertTrue(find_pointer_load(SMOKE_DISASSEMBLY))

    def test_real_fifo_call(self):
        self.assertTrue(find_rom_call(SMOKE_DISASSEMBLY))

    def test_wrong_pointer_offset_fails(self):
        changed = SMOKE_DISASSEMBLY.replace("-0x2c(a0)", "-0x28(a0)")
        self.assertFalse(find_pointer_load(changed))

    def test_wrong_rom_call_offset_fails(self):
        changed = SMOKE_DISASSEMBLY.replace("-0x77c(ra)", "-0x778(ra)")
        self.assertFalse(find_rom_call(changed))

    def test_symbols_without_instructions_are_insufficient(self):
        self.assertFalse(find_pointer_load("uart_acm_dev = 0x4FFBFFD4"))
        self.assertFalse(find_rom_call("esp_rom_cdc_acm_fifo_fill = 0x4FC008D0"))


class RomRxLinkTargetsTests(unittest.TestCase):
    def test_rx_count_and_read_targets(self):
        # 0x4ff50080 + (-0x34f << 12) - 0x7ac = 0x4fc008d4
        code = """
4ff50080:    auipc ra, 0xffcb1
4ff50084:    jalr -0x7ac(ra) <esp_rom_cdc_acm_rx_fifo_cnt>
4ff50088:    auipc ra, 0xffcb1
4ff5008c:    jalr -0x7b0(ra) <esp_rom_cdc_acm_fifo_read>
"""
        self.assertTrue(find_rom_call(code, ROM_RX_COUNT))
        self.assertTrue(find_rom_call(code, ROM_RX_READ))
        self.assertFalse(find_rom_call(code, ROM_RX_READ + 4))

    def test_rx_target_shift_is_rejected(self):
        code = """
4ff50080:    auipc ra, 0xffcb1
4ff50084:    jalr -0x7a8(ra)
"""
        self.assertFalse(find_rom_call(code, ROM_RX_COUNT))


class BringupAddressTests(unittest.TestCase):
    def test_mmio_store_uses_the_right_base_with_interleaved_instructions(self):
        code = """
4ff50a5c:    lui a0, 0x500e5
4ff50a60:    lw a1, 0x15c(a0)
4ff50a64:    ori a1, a1, 0x1
4ff50a68:    sw a1, 0x15c(a0)
"""
        self.assertEqual(find_mmio_accesses(code, 0x500E515C), {"lw", "sw"})
        changed = code.replace("ori a1,", "ori a0,")
        self.assertEqual(find_mmio_accesses(changed, 0x500E515C), {"lw"})
        self.assertFalse(find_mmio_accesses(code, 0x500E5160))

    def test_handler_uses_second_argument_register_and_expected_address(self):
        code = """
4ff50a86:    lui a1, 0x4fc01
4ff50a8a:    addi a1, a1, -0x68c
"""
        self.assertTrue(find_handler_address(code))
        self.assertFalse(find_handler_address(code.replace("-0x68c", "-0x680")))
        self.assertFalse(find_handler_address(code.replace("a1", "a2")))


if __name__ == "__main__":
    unittest.main()
