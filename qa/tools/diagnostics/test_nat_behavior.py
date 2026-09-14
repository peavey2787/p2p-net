"""Small offline codec/guard checks; does not contact a STUN service."""

import struct
import unittest

from nat_behavior import COOKIE, Diagnostic, decode


class StunChecks(unittest.TestCase):
    transaction = bytes(range(12))

    @classmethod
    def response(cls, transaction=None):
        value = struct.pack("!BBHI", 0, 1, 4567 ^ (COOKIE >> 16), 0xCB007101 ^ COOKIE)
        return (struct.pack("!HHI", 0x101, 12, COOKIE)
                + (transaction or cls.transaction) + struct.pack("!HH", 0x20, 8) + value)

    def test_xor_address(self):
        self.assertEqual(decode(self.response(), self.transaction)["mapped"], ("203.0.113.1", 4567))

    def test_unrelated_transaction_is_ignored(self):
        self.assertIsNone(decode(self.response(), bytes(12)))

    def test_malformed_lengths(self):
        packet = self.response()
        for bad in (packet[:-1], packet[:2] + bytes(2) + packet[4:],
                    packet[:22] + struct.pack("!H", 200) + packet[24:]):
            with self.subTest(packet=bad), self.assertRaises(ValueError):
                decode(bad, self.transaction)

    def test_alternate_must_be_public_and_have_different_ip_and_port(self):
        primary = ("8.8.8.8", 3478)
        self.assertEqual(Diagnostic.alternate({"other": ("8.8.4.4", 3479)}, primary), ("8.8.4.4", 3479))
        for other in (None, ("0.0.0.0", 3479), ("192.168.0.1", 3479),
                      ("8.8.8.8", 3479), ("8.8.4.4", 3478), ("8.8.4.4", 0)):
            with self.subTest(other=other), self.assertRaises(ValueError):
                Diagnostic.alternate({"other": other}, primary)

    def test_ignored_change_request_is_not_a_successful_filter_test(self):
        import time

        primary = ("8.8.8.8", 3478)

        class Socket:
            def getsockname(self):
                return ("0.0.0.0", 1234)

            def settimeout(self, _timeout):
                pass

            def sendto(self, packet, _target):
                self.transaction = packet[8:20]

            def recvfrom(self, _size):
                return StunChecks.response(self.transaction), primary

        with self.assertRaisesRegex(ValueError, "ignored requested response source"):
            Diagnostic(1, time.monotonic() + 5).binding(
                Socket(), primary, "change_test", 6, ("8.8.4.4", 3479))


if __name__ == "__main__":
    unittest.main()
