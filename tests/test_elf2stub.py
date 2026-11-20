import unittest
from tools.elf2stub import check_layout, IRAM, DRAM


class LayoutTests(unittest.TestCase):
    def test_valid(self):
        check_layout(IRAM[0], bytes(80), DRAM[0], 16, DRAM[0] + 16, bytes(4), IRAM[0])

    def test_overflow(self):
        with self.assertRaises(ValueError):
            check_layout(IRAM[0], bytes(IRAM[1] - IRAM[0] + 4), DRAM[0], 0, DRAM[0], b"", IRAM[0])

    def test_bss_collision(self):
        with self.assertRaises(ValueError):
            check_layout(IRAM[0], bytes(4), DRAM[0], 32, DRAM[0] + 8, bytes(4), IRAM[0])

    def test_invalid_entry(self):
        with self.assertRaises(ValueError):
            check_layout(IRAM[0], bytes(4), DRAM[0], 0, DRAM[0], b"", IRAM[1])

    def test_reserved_stack(self):
        with self.assertRaises(ValueError):
            check_layout(IRAM[0], bytes(4), DRAM[0], 0, DRAM[1], bytes(4), IRAM[0])


if __name__ == "__main__":
    unittest.main()
