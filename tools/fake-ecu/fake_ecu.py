#!/usr/bin/env python3
"""Fake Speeduino ECU for developing rustytune without hardware.

Vendored from ../speeduino-fake-ecu (the standalone project) and extended:
primary mode speaks the enveloped Protocol 3 command set of the 202501
firmware (verified against speeduino/speeduino comms.cpp, whose dispatch is
identical from 202402.3 through 202501.7):

  'A'            legacy realtime block
  'r'            och realtime read ('r' + canId + 0x30 + off(LE16) + cnt(LE16))
  'p'            page read  ('p' + canId + page + off(LE16) + cnt(LE16))
  'M'            page chunk write (same header + value bytes)
  'b'/'B'        burn page to "EEPROM" -> replies SERIAL_RC_BURN_OK (0x04)
  'd'            page CRC32 -> 0x00 + CRC32 big-endian
  'Q'            code version ("speeduino 202501")
  'S'            product string ("Speeduino 2025.01")
  'C'            comms test -> 0x00 0xFF
  'f'            capabilities -> proto version + blocking factors (BE16)

Incoming envelope CRCs are validated (bad requests are dropped, like the
firmware). Out-of-range page reads/writes reply SERIAL_RC_RANGE_ERR (0x84),
unknown commands SERIAL_RC_UKWN_ERR (0x83).

Pages hold a deterministic default pattern (byte i of page n, 1-based, is
(n * 31 + i) & 0xFF). 'M' writes hit the working copy; 'b' copies the page
to the burned set and — with --storage PATH — persists it to disk, so burns
survive a simulator restart exactly like EEPROM.

Secondary mode is unchanged: raw 'r'/0x30 telemetry only, no checksum.

Telemetry values sweep smoothly by default so gauges visibly move; use
--static for the fixed reference values checked by the test suite. Channel
offsets match the Speeduino modern-layout defaults (rustytune's fixture INI).
"""

import argparse
import json
import math
import os
import pty
import select
import struct
import sys
import termios
import time
import tty
import zlib

OCH_OFFSETS = {  # keep in sync with fixtures/speeduino202501_7.ini
    "engine": 2,  # bitfield: 0 running, 1 crank, 3 warmup
    "map": 4,  # U16
    "iat": 6,
    "clt": 7,
    "bat": 9,
    "afr": 10,
    "rpm": 14,  # U16
    "spark": 24,
    "tps": 25,
}

# fixtures/speeduino202501_7.ini pageSize list (pages 1..15).
PAGE_SIZES = [128, 288, 288, 128, 288, 128, 240, 384, 192, 192, 288, 192, 128, 288, 256]

SIGNATURE = b"speeduino 202501"       # 'Q' response, matches INI signature
PRODUCT_STRING = b"Speeduino 2025.01"  # 'S' response
BLOCKING_FACTOR = 251
TABLE_BLOCKING_FACTOR = 256

RC_OK = 0x00
RC_BURN_OK = 0x04
RC_UKWN_ERR = 0x83
RC_RANGE_ERR = 0x84

# Payload length constraints per command, for envelope resync: (min, max).
CMD_LENGTHS = {
    ord("A"): (1, 1),
    ord("r"): (7, 7),
    ord("p"): (7, 7),
    ord("M"): (8, 7 + BLOCKING_FACTOR),
    ord("b"): (3, 3),
    ord("B"): (3, 3),
    ord("d"): (3, 3),
    ord("Q"): (1, 1),
    ord("S"): (1, 1),
    ord("C"): (1, 1),
    ord("f"): (1, 1),
}

BAUD_CONSTANTS = {
    9600: termios.B9600,
    19200: termios.B19200,
    38400: termios.B38400,
    57600: termios.B57600,
    115200: termios.B115200,
    230400: termios.B230400,
}


def default_page(page_num):
    """Deterministic default content for 1-based page `page_num`."""
    size = PAGE_SIZES[page_num - 1]
    return bytearray((page_num * 31 + i) & 0xFF for i in range(size))


class PageStore:
    """Working ("RAM") and burned ("EEPROM") copies of every tune page.

    Burned pages persist to --storage as JSON hex; on startup the working
    copy is initialized from the burned state, like a real power-on.
    """

    def __init__(self, storage_path):
        self.storage_path = storage_path
        self.burned = {n: default_page(n) for n in range(1, len(PAGE_SIZES) + 1)}
        if storage_path and os.path.exists(storage_path):
            with open(storage_path) as f:
                saved = json.load(f)
            for key, hexdata in saved.get("pages", {}).items():
                num = int(key)
                data = bytearray.fromhex(hexdata)
                if 1 <= num <= len(PAGE_SIZES) and len(data) == PAGE_SIZES[num - 1]:
                    self.burned[num] = data
        self.working = {n: bytearray(p) for n, p in self.burned.items()}

    def valid(self, page_num):
        return 1 <= page_num <= len(PAGE_SIZES)

    def read(self, page_num, offset, count):
        page = self.working[page_num]
        if offset + count > len(page):
            return None
        return bytes(page[offset : offset + count])

    def write(self, page_num, offset, data):
        page = self.working[page_num]
        if offset + len(data) > len(page):
            return False
        page[offset : offset + len(data)] = data
        return True

    def crc(self, page_num):
        return zlib.crc32(bytes(self.working[page_num]))

    def burn(self, page_num):
        self.burned[page_num] = bytearray(self.working[page_num])
        if self.storage_path:
            doc = {"pages": {str(n): p.hex() for n, p in self.burned.items()}}
            tmp = self.storage_path + ".tmp"
            with open(tmp, "w") as f:
                json.dump(doc, f)
            os.replace(tmp, self.storage_path)


def build_payload(och_size, t, static):
    """Fill an och block. `t` is seconds since start."""
    if static:
        rpm, mapv, tps_raw = 3450, 98, 44  # tps 22.0% at 0.5 scale
        clt_raw, iat_raw = 130, 65  # 90 C / 25 C after the -40 offset
        afr_raw, bat_raw, adv = 147, 139, 18  # 14.7 / 13.9 V
    else:
        sweep = (math.sin(t * 0.5) + 1) / 2  # 0..1, ~12.5 s period
        rpm = int(900 + sweep * 5600)
        tps_raw = int(sweep * 200)  # 0-100% at 0.5 scale
        mapv = int(30 + sweep * 70)
        clt_raw = min(90, int(20 + t * 2)) + 40  # warms up to 90 C
        iat_raw = 25 + 40 + int(3 * math.sin(t * 0.1))
        afr_raw = 147 + int(8 * math.sin(t * 1.3))
        bat_raw = 139 + int(2 * math.sin(t * 0.3))
        adv = int(10 + sweep * 25)

    payload = bytearray(och_size)
    struct.pack_into("<H", payload, OCH_OFFSETS["rpm"], rpm)
    struct.pack_into("<H", payload, OCH_OFFSETS["map"], mapv)
    payload[OCH_OFFSETS["tps"]] = tps_raw
    payload[OCH_OFFSETS["clt"]] = clt_raw
    payload[OCH_OFFSETS["iat"]] = iat_raw
    payload[OCH_OFFSETS["afr"]] = afr_raw
    payload[OCH_OFFSETS["bat"]] = bat_raw
    payload[OCH_OFFSETS["spark"]] = adv
    # Engine status bits so indicator lamps light: running above cranking
    # speed, warmup until coolant reaches operating temperature.
    engine = 0x01 if rpm > 400 else 0x02
    if clt_raw - 40 < 90:
        engine |= 0x08
    payload[OCH_OFFSETS["engine"]] = engine
    return payload


def open_port(args):
    """Return (fd, cleanup_fn)."""
    if args.device:
        fd = os.open(args.device, os.O_RDWR | os.O_NOCTTY)
        tty.setraw(fd)
        attrs = termios.tcgetattr(fd)
        speed = BAUD_CONSTANTS[args.baud]
        attrs[4] = attrs[5] = speed  # ispeed, ospeed
        termios.tcsetattr(fd, termios.TCSANOW, attrs)
        print(f"fake ECU on {args.device} @ {args.baud}", flush=True)
        return fd, lambda: os.close(fd)

    master, slave = pty.openpty()
    if os.path.islink(args.link) or os.path.exists(args.link):
        os.unlink(args.link)
    os.symlink(os.ttyname(slave), args.link)
    print(f"fake ECU on {os.ttyname(slave)} -> {args.link}", flush=True)

    def cleanup():
        os.close(master)
        os.close(slave)
        try:
            os.unlink(args.link)
        except OSError:
            pass

    return master, cleanup


def envelope(body):
    return struct.pack(">H", len(body)) + body + struct.pack(">I", zlib.crc32(body))


def rc_msg(code):
    return envelope(bytes([code]))


def dispatch(payload, store, och_size, t, static):
    """One validated envelope payload -> response envelope (or None)."""
    cmd = payload[0:1]

    if cmd == b"A":
        return envelope(bytes([RC_OK]) + bytes(build_payload(och_size, t, static)))

    if cmd == b"r":
        # 'r' + canId + type + offset(LE16) + count(LE16); type 0x30 = och
        if payload[2] != 0x30:
            return rc_msg(RC_UKWN_ERR)
        count = payload[5] | (payload[6] << 8)
        data = build_payload(och_size, t, static)[:count]
        return envelope(bytes([RC_OK]) + bytes(data))

    if cmd in (b"p", b"M", b"b", b"B", b"d"):
        page_num = payload[2]  # payload[1] is canId (unused, like firmware)
        if not store.valid(page_num):
            return rc_msg(RC_RANGE_ERR)

        if cmd == b"p":
            offset = payload[3] | (payload[4] << 8)
            count = payload[5] | (payload[6] << 8)
            data = store.read(page_num, offset, count)
            if data is None:
                return rc_msg(RC_RANGE_ERR)
            return envelope(bytes([RC_OK]) + data)

        if cmd == b"M":
            offset = payload[3] | (payload[4] << 8)
            count = payload[5] | (payload[6] << 8)
            values = payload[7:]
            if count != len(values) or not store.write(page_num, offset, values):
                return rc_msg(RC_RANGE_ERR)
            return rc_msg(RC_OK)

        if cmd == b"d":
            return envelope(bytes([RC_OK]) + struct.pack(">I", store.crc(page_num)))

        # 'b'/'B': firmware acks with BURN_OK, not OK
        store.burn(page_num)
        return rc_msg(RC_BURN_OK)

    if cmd == b"Q":
        return envelope(bytes([RC_OK]) + SIGNATURE)
    if cmd == b"S":
        return envelope(bytes([RC_OK]) + PRODUCT_STRING)
    if cmd == b"C":
        return envelope(bytes([RC_OK, 0xFF]))
    if cmd == b"f":
        return envelope(
            bytes([RC_OK, 2])
            + struct.pack(">H", BLOCKING_FACTOR)
            + struct.pack(">H", TABLE_BLOCKING_FACTOR)
        )

    return rc_msg(RC_UKWN_ERR)


def handle_primary(buf, store, och_size, t, static):
    """Try to consume one enveloped command from buf.

    Returns (new_buf, response_bytes_or_None, made_progress).
    """
    if len(buf) < 3:
        return buf, None, False
    plen = (buf[0] << 8) | buf[1]
    limits = CMD_LENGTHS.get(buf[2])
    if limits is None or not limits[0] <= plen <= limits[1]:
        return buf[1:], None, True  # implausible header: resync one byte
    need = 2 + plen + 4
    if len(buf) < need:
        return buf, None, False  # wait for the rest of the frame

    payload = buf[2 : 2 + plen]
    (crc_wire,) = struct.unpack(">I", buf[2 + plen : need])
    buf = buf[need:]
    if zlib.crc32(payload) != crc_wire:
        return buf, None, True  # bad request CRC: drop, like the firmware

    return buf, dispatch(payload, store, och_size, t, static), True


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--mode", choices=["secondary", "primary"], default="secondary",
        help="ECU protocol to speak (default secondary, like the SER3 header)")
    parser.add_argument(
        "--link", default="./fakeecu",
        help="symlink to create for the pty (default ./fakeecu)")
    parser.add_argument(
        "--device", metavar="TTY",
        help="serve on an existing serial device instead of a pty "
             "(e.g. a USB adapter cross-wired to the Pi)")
    parser.add_argument("--baud", type=int, default=115200,
                        choices=sorted(BAUD_CONSTANTS), help="baud for --device")
    parser.add_argument("--och-size", type=int, default=130,
                        help="och block size, matches fixture ini (default 130)")
    parser.add_argument("--duration", type=float, default=0,
                        help="exit after N seconds (default: run until killed)")
    parser.add_argument("--static", action="store_true",
                        help="fixed reference values instead of animated sweeps")
    parser.add_argument(
        "--storage", metavar="PATH",
        help="persist burned pages to this JSON file (primary mode); burns "
             "survive restarts like EEPROM")
    parser.add_argument(
        "--corrupt-every", type=int, metavar="N", default=0,
        help="corrupt one byte of every Nth response (primary mode: provokes "
             "CRC-mismatch drops; secondary mode: silently wrong values, as "
             "that protocol has no checksum)")
    args = parser.parse_args()

    store = PageStore(args.storage)
    fd, cleanup = open_port(args)
    start = time.monotonic()
    deadline = start + args.duration if args.duration > 0 else None
    buf = b""
    polls = 0

    try:
        while deadline is None or time.monotonic() < deadline:
            ready, _, _ = select.select([fd], [], [], 0.1)
            if not ready:
                continue
            try:
                data = os.read(fd, 256)
            except OSError:
                break
            if not data:
                break
            buf += data

            while buf:
                resp = None
                t = time.monotonic() - start
                if args.mode == "secondary":
                    if len(buf) < 7:
                        break
                    # 'r' + canId + 0x30 + offset(LE16) + count(LE16)
                    if not (buf[0:1] == b"r" and buf[2] == 0x30):
                        buf = buf[1:]  # resync
                        continue
                    count = buf[5] | (buf[6] << 8)
                    buf = buf[7:]
                    payload = build_payload(args.och_size, t, args.static)
                    resp = b"r" + bytes([0x30]) + bytes(payload[:count])
                else:
                    buf, resp, progressed = handle_primary(
                        buf, store, args.och_size, t, args.static)
                    if not progressed:
                        break
                    if resp is None:
                        continue

                polls += 1
                if args.corrupt_every and polls % args.corrupt_every == 0:
                    resp = bytearray(resp)
                    resp[len(resp) // 2] ^= 0xFF
                    resp = bytes(resp)
                os.write(fd, resp)
    except KeyboardInterrupt:
        pass
    finally:
        cleanup()
        print(f"answered {polls} polls", flush=True)


if __name__ == "__main__":
    sys.exit(main())
