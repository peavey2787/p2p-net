"""Two-host public-WAN UDP hole-punch diagnostic with a 60-second ceiling.

This is intentionally separate from the production node. It exchanges only a
public STUN mapping and a random per-run cookie through a shared directory,
then sends authenticated probe datagrams across the public UDP port range from
the same socket. It never treats a private/LAN address as success.
"""

import argparse
import ipaddress
import json
import os
from pathlib import Path
import secrets
import select
import socket
import struct
import threading
import time


COOKIE = 0x2112A442
MAGIC = b"p2p-net-symmetric-punch-v1\0"


def stun_mapping(sock, server, port, deadline):
    target = (socket.gethostbyname(server), port)
    transaction = secrets.token_bytes(12)
    request = struct.pack("!HHI", 1, 0, COOKIE) + transaction
    while time.monotonic() < deadline:
        sock.sendto(request, target)
        until = min(deadline, time.monotonic() + 1.0)
        while time.monotonic() < until:
            sock.settimeout(max(0.001, until - time.monotonic()))
            try:
                packet, _ = sock.recvfrom(2048)
            except (socket.timeout, ConnectionResetError):
                break
            if len(packet) < 20 or packet[8:20] != transaction:
                continue
            length = struct.unpack_from("!H", packet, 2)[0]
            offset = 20
            while offset < min(len(packet), 20 + length):
                attr, size = struct.unpack_from("!HH", packet, offset)
                value = packet[offset + 4:offset + 4 + size]
                if attr == 0x20 and len(value) == 8 and value[:2] == b"\0\x01":
                    mapped_port, mapped_ip = struct.unpack("!HI", value[2:])
                    mapped_port ^= COOKIE >> 16
                    mapped_ip ^= COOKIE
                    return str(ipaddress.IPv4Address(mapped_ip)), mapped_port
                offset = (offset + 4 + size + 3) & ~3
    raise TimeoutError("STUN mapping timed out")


def atomic_json(path, value):
    temp = path.with_suffix(path.suffix + ".tmp")
    temp.write_text(json.dumps(value), encoding="utf-8")
    os.replace(temp, path)


def wait_peer(path, session, deadline):
    while time.monotonic() < deadline:
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
            if value.get("session") == session:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(0.02)
    raise TimeoutError("peer exchange timed out")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--role", choices=("alice", "bob"), required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--exchange", type=Path, required=True)
    parser.add_argument("--local-port", type=int, default=49191)
    parser.add_argument("--timeout-secs", type=int, default=60)
    parser.add_argument("--stun", default="stun.voipgate.com")
    parser.add_argument("--fanout-role", choices=("alice", "bob"), default="alice")
    parser.add_argument("--fanout-sockets", type=int, default=128)
    parser.add_argument("--port-guesses", type=int, default=2048)
    parser.add_argument("--guess-delay-ms", type=float, default=5.0)
    args = parser.parse_args()
    if args.timeout_secs != 60:
        parser.error("this acceptance diagnostic requires --timeout-secs 60")

    started = time.monotonic()
    deadline = started + args.timeout_secs
    args.exchange.mkdir(parents=True, exist_ok=True)
    mine = args.exchange / f"{args.role}.udp.json"
    other_role = "bob" if args.role == "alice" else "alice"
    other = args.exchange / f"{other_role}.udp.json"
    result = args.exchange / f"{args.role}.udp-result.json"
    token = secrets.token_hex(16)

    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("0.0.0.0", args.local_port))
        public_ip, public_port = stun_mapping(
            sock, args.stun, 3478, min(deadline, time.monotonic() + 10)
        )
        if not ipaddress.ip_address(public_ip).is_global:
            raise RuntimeError("STUN did not return a public IPv4 address")
        atomic_json(mine, {
            "session": args.session,
            "role": args.role,
            "token": token,
            "public_ip": public_ip,
            "public_port": public_port,
        })
        peer = wait_peer(other, args.session, deadline)
        peer_ip = peer["public_ip"]
        if not ipaddress.ip_address(peer_ip).is_global:
            raise RuntimeError("refusing to punch a non-public peer address")

        punch = MAGIC + args.session.encode() + b"\0" + token.encode()
        expected = MAGIC + args.session.encode() + b"\0" + peer["token"].encode()
        ack = b"ack\0" + expected
        expected_ack = b"ack\0" + punch
        extra_sockets = []
        if args.role == args.fanout_role:
            for _ in range(args.fanout_sockets):
                candidate = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                candidate.bind(("0.0.0.0", 0))
                candidate.setblocking(False)
                extra_sockets.append(candidate)
        receive_sockets = [sock, *extra_sockets]
        stop = threading.Event()
        received = {}

        def receiver():
            while not stop.is_set() and time.monotonic() < deadline:
                try:
                    ready, _, _ = select.select(receive_sockets, [], [], 0.05)
                except (ValueError, OSError):
                    continue
                for source_socket in ready:
                    try:
                        packet, source = source_socket.recvfrom(2048)
                    except (BlockingIOError, ConnectionResetError, OSError):
                        continue
                    if source[0] != peer_ip:
                        continue
                    if packet == expected:
                        received["source"] = source
                        for _ in range(32):
                            try:
                                source_socket.sendto(ack, source)
                            except OSError:
                                pass
                        received["punch"] = True
                    elif packet == expected_ack:
                        received["source"] = source
                        received["ack"] = True
                if received.get("punch") and received.get("ack"):
                    stop.set()

        thread = threading.Thread(target=receiver, daemon=True)
        thread.start()
        start_port = secrets.randbelow(65535) + 1
        sent = 0
        while time.monotonic() < deadline and not stop.is_set():
            if extra_sockets:
                # Mirror the QUIC DCUtR dialer: many real source mappings send
                # repeatedly to the peer's single endpoint-independent mapping.
                for candidate in extra_sockets:
                    try:
                        candidate.sendto(punch, (peer_ip, peer["public_port"]))
                    except (ConnectionResetError, OSError):
                        pass
                    sent += 1
                time.sleep(0.2)
                continue
            # Mirror the listener-side bounded birthday sample. 251 is coprime
            # with 65,535, so guesses do not repeat within a pass.
            for offset in range(args.port_guesses):
                port = ((start_port - 1 + offset * 251) % 65535) + 1
                try:
                    sock.sendto(punch, (peer_ip, port))
                except (ConnectionResetError, OSError):
                    pass
                sent += 1
                if stop.is_set() or time.monotonic() >= deadline:
                    break
                if args.guess_delay_ms > 0:
                    time.sleep(args.guess_delay_ms / 1000.0)
            # Repeat the same bounded set to keep filters alive without
            # allocating fresh conntrack entries on every pass.
        if received.get("source"):
            until = min(deadline, time.monotonic() + 1.0)
            while time.monotonic() < until:
                try:
                    sock.sendto(ack, tuple(received["source"]))
                except OSError:
                    pass
                time.sleep(0.01)
        stop.set()
        thread.join(timeout=0.2)
        for candidate in extra_sockets:
            candidate.close()

    value = {
        "session": args.session,
        "role": args.role,
        "success": bool(received.get("source")),
        "bidirectional": bool(received.get("punch") and received.get("ack")),
        "local_public": [public_ip, public_port],
        "peer_stun": [peer_ip, peer["public_port"]],
        "direct_source": list(received["source"]) if received.get("source") else None,
        "datagrams_sent": sent,
        "elapsed_ms": int((time.monotonic() - started) * 1000),
    }
    atomic_json(result, value)
    print(json.dumps(value), flush=True)
    return 0 if value["success"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
