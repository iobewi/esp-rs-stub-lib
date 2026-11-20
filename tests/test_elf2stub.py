import unittest
from tools.elf2stub import (
    BSS_SENTINEL_SIZE,
    DATA_SENTINEL,
    DRAM,
    IRAM,
    check_entry,
    check_layout,
    check_sentinel,
)


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

    def test_entry_must_match_esp_main(self):
        with self.assertRaises(ValueError):
            check_entry(IRAM[0] + 4, IRAM[0], IRAM[0], 16)
        with self.assertRaises(ValueError):
            check_entry(IRAM[0] + 1, IRAM[0] + 1, IRAM[0], 16)

    def test_sentinel_payload(self):
        class Section:
            def __init__(self, addr, payload):
                self.meta = {"sh_addr": addr, "sh_size": len(payload)}
                self.payload = payload

            def __getitem__(self, key):
                return self.meta[key]

            def data(self):
                return self.payload

        section = Section(DRAM[0], DATA_SENTINEL)
        check_sentinel("INITIALIZED_SENTINEL",
                       {"st_value": DRAM[0], "st_size": len(DATA_SENTINEL)},
                       section, len(DATA_SENTINEL), DATA_SENTINEL)
        with self.assertRaises(ValueError):
            check_sentinel("INITIALIZED_SENTINEL",
                           {"st_value": DRAM[0], "st_size": len(DATA_SENTINEL)},
                           Section(DRAM[0], b"BAD!"),
                           len(DATA_SENTINEL), DATA_SENTINEL)
        self.assertEqual(BSS_SENTINEL_SIZE, 16)


if __name__ == "__main__":
    unittest.main()
