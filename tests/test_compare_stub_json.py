import base64
import unittest
from tools.compare_stub_json import compare


def stub():
    return {
        "entry": 0x4FF50000, "text_start": 0x4FF50000,
        "data_start": 0x4FF60010, "bss_start": 0x4FF60000,
        "text": base64.b64encode(b"code").decode(),
        "data": base64.b64encode(b"data").decode(),
    }


class ComparisonTests(unittest.TestCase):
    def test_equal(self):
        a = stub()
        self.assertEqual(compare(a, a.copy())["text"]["length"], 4)

    def test_entry_change(self):
        a, b = stub(), stub()
        b["entry"] += 2
        with self.assertRaisesRegex(ValueError, "entry"):
            compare(a, b)

    def test_address_change(self):
        a, b = stub(), stub()
        b["data_start"] += 4
        with self.assertRaisesRegex(ValueError, "data_start"):
            compare(a, b)

    def test_text_change(self):
        a, b = stub(), stub()
        b["text"] = base64.b64encode(b"cod!").decode()
        with self.assertRaisesRegex(ValueError, "text"):
            compare(a, b)

    def test_data_change(self):
        a, b = stub(), stub()
        b["data"] = base64.b64encode(b"dat!").decode()
        with self.assertRaisesRegex(ValueError, "data"):
            compare(a, b)

    def test_extra_field(self):
        a, b = stub(), stub()
        b["x"] = 1
        with self.assertRaisesRegex(ValueError, "fields"):
            compare(a, b)
