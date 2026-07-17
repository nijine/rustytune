#!/usr/bin/env python3
"""Fake Speeduino ECU for developing rustytune without hardware.

Vendored from ../speeduino-fake-ecu (the standalone project) and extended:
primary mode now also answers the INI's real telemetry command — an
enveloped 'r' och read — in addition to the legacy bare 'A', and incoming
envelope CRCs are validated (bad requests are dropped, like the firmware).

By default it creates a pty and a symlink to it (./fakeecu), then answers
telemetry polls on it:

  secondary mode: 'r' + canId + 0x30 + offset(LE16) + count(LE16)
                  -> 'r' + 0x30 + <count raw och bytes>
  primary mode:   len(BE16) + payload + CRC32(BE32) where payload is
                  'A'                                   (legacy realtime)
                  'r' + canId + 0x30 + off(LE16) + count(LE16)
                  -> len(BE16) + 0x00 + <data> + CRC32(BE32)

Telemetry values sweep smoothly by default so gauges visibly move; use
--static for the fixed reference values checked by the test suite. Channel
offsets match the Speeduino modern-layout defaults (rustytune's fixture INI).
"""

import argparse
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

OCH_OFFSETS = {  # keep in sync with fixtures/speeduino202405_dev.ini
    "map": 4,  # U16
    "iat": 6,
    "clt": 7,
    "bat": 9,
    "afr": 10,
    "rpm": 14,  # U16
    "spark": 24,
    "tps": 25,
}

BAUD_CONSTANTS = {
    9600: termios.B9600,
    19200: termios.B19200,
    38400: termios.B38400,
    57600: termios.B57600,
    115200: termios.B115200,
    230400: termios.B230400,
}


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


def handle_primary(buf, och_size, t, static):
    """Try to consume one enveloped command from buf.

    Returns (new_buf, response_bytes_or_None, made_progress).
    """
    if len(buf) < 2:
        return buf, None, False
    plen = (buf[0] << 8) | buf[1]
    known = (plen == 1 and len(buf) >= 3 and buf[2] == 0x41) or (
        plen == 7 and len(buf) >= 5 and buf[2:3] == b"r" and buf[4] == 0x30
    )
    if not known:
        return buf[1:], None, True  # resync one byte
    need = 2 + plen + 4
    if len(buf) < need:
        return buf, None, False  # wait for the rest of the frame

    payload = buf[2 : 2 + plen]
    (crc_wire,) = struct.unpack(">I", buf[2 + plen : need])
    buf = buf[need:]
    if zlib.crc32(payload) != crc_wire:
        return buf, None, True  # bad request CRC: drop, like the firmware

    data = build_payload(och_size, t, static)
    if payload[0:1] == b"r":
        count = payload[5] | (payload[6] << 8)
        data = data[:count]
    return buf, envelope(bytes([0x00]) + bytes(data)), True


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
    parser.add_argument("--och-size", type=int, default=127,
                        help="och block size, matches fixture ini (default 127)")
    parser.add_argument("--duration", type=float, default=0,
                        help="exit after N seconds (default: run until killed)")
    parser.add_argument("--static", action="store_true",
                        help="fixed reference values instead of animated sweeps")
    parser.add_argument(
        "--corrupt-every", type=int, metavar="N", default=0,
        help="corrupt one byte of every Nth response (primary mode: provokes "
             "CRC-mismatch drops; secondary mode: silently wrong values, as "
             "that protocol has no checksum)")
    args = parser.parse_args()

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
                        buf, args.och_size, t, args.static)
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
