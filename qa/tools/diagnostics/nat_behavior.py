"""Bounded IPv4 NAT diagnostic; not a libp2p/DCUtR acceptance test.

Uses RFC 5780 binding/change requests to one public STUN service. No port
scanning, router changes, or relay allocations. Mapping and filtering use
separate fresh sockets. Server/source checks prevent ignored CHANGE-REQUESTs
and unusable OTHER-ADDRESS responses from being classified as NAT behavior.

The default is the public server used by Pion's stun-nat-behaviour example.
Run on each host: python qa/tools/diagnostics/nat_behavior.py --tcp-mapping
The optional TCP check observes mappings only, not TCP filtering behavior.
"""

import argparse
from contextlib import ExitStack
import ipaddress
import json
import secrets
import socket
import struct
import time


COOKIE = 0x2112A442


def address(value, xor=False):
    if len(value) != 8 or value[:2] != b"\x00\x01":
        raise ValueError("expected an IPv4 STUN address")
    port, ip = struct.unpack("!HI", value[2:])
    if xor:
        port ^= COOKIE >> 16
        ip ^= COOKIE
    return (str(ipaddress.IPv4Address(ip)), port)


def decode(packet, transaction):
    if len(packet) < 20 or packet[8:20] != transaction:
        return None
    kind, length, cookie = struct.unpack("!HHI", packet[:8])
    if cookie != COOKIE or length % 4 or len(packet) != 20 + length:
        raise ValueError("invalid STUN header/length")
    if kind not in (0x101, 0x111):
        raise ValueError("not a binding response")
    result = {"response_type": kind}
    offset = 20
    while offset < len(packet):
        attr, size = struct.unpack_from("!HH", packet, offset)
        end = offset + 4 + size
        if end > len(packet):
            raise ValueError("truncated STUN attribute")
        value = packet[offset + 4:end]
        if attr in (0x20, 0x802B, 0x802C):
            result[{0x20: "mapped", 0x802B: "origin", 0x802C: "other"}[attr]] = address(value, attr == 0x20)
        if attr == 9 and len(value) >= 4:
            result["error"] = (value[2] & 7) * 100 + value[3]
        offset = (end + 3) & ~3
    if kind == 0x111:
        raise ValueError(f"STUN server error: {result.get('error', 'unknown')}")
    return result


class Diagnostic:
    def __init__(self, timeout, deadline):
        self.timeout = timeout
        self.deadline = deadline
        self.steps = []

    def binding(self, sock, target, label, change=0, expected=None):
        expected = expected or target
        transaction = secrets.token_bytes(12)
        attrs = struct.pack("!HHI", 3, 4, change) if change else b""
        packet = struct.pack("!HHI", 1, len(attrs), COOKIE) + transaction + attrs
        step = {"step": label, "local_port": sock.getsockname()[1],
                "target": target, "expected_source": expected, "change": change}
        self.steps.append(step)
        # Two identical transmissions reduce the effect of a single lost UDP
        # packet without multiplying endpoints or exceeding the run deadline.
        for _ in range(2):
            if time.monotonic() >= self.deadline:
                raise TimeoutError("60-second diagnostic deadline")
            sock.sendto(packet, target)
            until = min(self.deadline, time.monotonic() + self.timeout)
            while time.monotonic() < until:
                sock.settimeout(max(0.001, until - time.monotonic()))
                try:
                    data, source = sock.recvfrom(2048)
                except socket.timeout:
                    break
                response = decode(data, transaction)
                if response is None:
                    continue
                step.update(response, source=source)
                if source != expected:
                    raise ValueError(f"{label}: server ignored requested response source")
                if response.get("origin", source) != source:
                    raise ValueError(f"{label}: RESPONSE-ORIGIN/source mismatch")
                if "mapped" not in response:
                    raise ValueError(f"{label}: missing XOR-MAPPED-ADDRESS")
                return response
        step["timeout"] = True
        return None

    @staticmethod
    def alternate(response, primary):
        other = response.get("other")
        if (not other or not ipaddress.ip_address(other[0]).is_global
                or not other[1] or other[0] == primary[0] or other[1] == primary[1]):
            raise ValueError("server has no usable two-IP/two-port OTHER-ADDRESS")
        return other

    def filtering(self, primary):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("0.0.0.0", 0))
            first = self.binding(sock, primary, "filter_binding")
            if first is None:
                raise ValueError("no primary binding response")
            other = self.alternate(first, primary)
            if self.binding(sock, primary, "filter_change_ip_port", 6, other):
                return "endpoint_independent"
            if self.binding(sock, primary, "filter_change_port", 2, (primary[0], other[1])):
                result = "address_dependent"
            else:
                result = "address_and_port_dependent"
            # RFC 5780 reported erratum 7971: do not mistake an unreachable
            # alternate server for restrictive client filtering. Also test
            # both alternate ports; these checks happen AFTER filtering.
            same_mapping = True
            for target in ((other[0], primary[1]), other, (primary[0], other[1])):
                response = self.binding(sock, target, "alternate_reachability")
                if response is None:
                    raise ValueError("alternate service unreachable; filtering inconclusive")
                same_mapping = same_mapping and response["mapped"] == first["mapped"]
            # With an unchanged mapping, all alternate return paths have now
            # been opened. Successful change responses here distinguish actual
            # filtering from a server that silently drops CHANGE-REQUESTs.
            if same_mapping:
                for change, source in ((6, other), (2, (primary[0], other[1]))):
                    if self.binding(sock, primary, "change_support_after_priming", change, source) is None:
                        raise ValueError("CHANGE-REQUEST support unverified; filtering inconclusive")
            return result

    def mapping(self, primary):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("0.0.0.0", 0))
            first = self.binding(sock, primary, "mapping_binding")
            if first is None:
                raise ValueError("no primary binding response")
            other = self.alternate(first, primary)
            second = self.binding(sock, (other[0], primary[1]), "mapping_other_ip")
            if second is None:
                raise ValueError("no alternate binding response")
            if second["mapped"] == first["mapped"]:
                return "endpoint_independent"
            third = self.binding(sock, other, "mapping_other_ip_port")
            if third is None:
                raise ValueError("no alternate-port binding response")
            return ("address_dependent" if third["mapped"] == second["mapped"]
                    else "address_and_port_dependent")

    def tcp_mapping(self, primary):
        """Observe TCP mappings from one listening port; no filtering claim."""
        first = next((s for s in self.steps if "other" in s), {})
        other = self.alternate(first, primary)
        observations = []
        with ExitStack() as stack:
            def reused_socket():
                sock = stack.enter_context(socket.socket(socket.AF_INET, socket.SOCK_STREAM))
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                if hasattr(socket, "SO_REUSEPORT"):
                    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
                return sock

            def remaining(sock):
                budget = self.deadline - time.monotonic()
                if budget <= 0:
                    raise TimeoutError("60-second diagnostic deadline")
                sock.settimeout(min(3, budget))

            listener = reused_socket()
            listener.bind(("0.0.0.0", 0))
            listener.listen()
            port = listener.getsockname()[1]
            # Keep all three distinct-destination sockets alive, preserving the
            # shared source port without reusing an identical TCP four-tuple.
            for target in (primary, (other[0], primary[1]), other):
                step = {"step": "tcp_mapping", "target": target, "local_port": port}
                self.steps.append(step)
                sock = reused_socket()
                sock.bind(("0.0.0.0", port))
                remaining(sock)
                sock.connect(target)
                step["local"] = sock.getsockname()
                transaction = secrets.token_bytes(12)
                remaining(sock)
                sock.sendall(struct.pack("!HHI", 1, 0, COOKIE) + transaction)
                packet = b""
                expected = 20
                while len(packet) < expected:
                    remaining(sock)
                    data = sock.recv(expected - len(packet))
                    if not data:
                        raise ValueError("TCP STUN server closed before a complete response")
                    packet += data
                    if len(packet) >= 20:
                        expected = 20 + struct.unpack_from("!H", packet, 2)[0]
                        if expected > 2048:
                            raise ValueError("oversized TCP STUN response")
                response = decode(packet, transaction)
                if response is None or "mapped" not in response:
                    raise ValueError("invalid TCP STUN binding response")
                step.update(response)
                observations.append(response["mapped"])
        return observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", default="stun.voipgate.com")
    parser.add_argument("--port", type=int, default=3478)
    parser.add_argument("--timeout", type=float, default=2)
    parser.add_argument("--tcp-mapping", action="store_true",
                        help="also measure three TCP mappings on the advertised STUN endpoints")
    args = parser.parse_args()
    if not 0 < args.timeout <= 3 or not 0 < args.port <= 65535:
        parser.error("timeout must be in (0, 3], port in [1, 65535]")
    diagnostic = Diagnostic(args.timeout, time.monotonic() + 60)
    output = {"server": args.server, "port": args.port,
              "scope": "Python socket path observations; not DCUtR acceptance"}
    checks = ("filtering", "mapping", "tcp_mapping") if args.tcp_mapping else ("filtering", "mapping")
    try:
        primary = (socket.gethostbyname(args.server), args.port)
        if not ipaddress.ip_address(primary[0]).is_global:
            raise ValueError("a public IPv4 STUN server is required")
        output["primary"] = primary
        for name in checks:
            try:
                output[name] = getattr(diagnostic, name)(primary)
            except (OSError, ValueError) as error:
                output[name] = "inconclusive"
                output[name + "_error"] = str(error)
    except (OSError, ValueError) as error:
        output["error"] = str(error)
    output["steps"] = diagnostic.steps
    print(json.dumps(output, indent=2), flush=True)
    return 0 if all(output.get(k) not in (None, "inconclusive") for k in checks) else 2


if __name__ == "__main__":
    raise SystemExit(main())
