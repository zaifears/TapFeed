"""Write a Tapo recording as it streams in, counting received seconds from the video PTS."""

from __future__ import annotations

from typing import BinaryIO, Dict, Optional

VIDEO, AUDIO = 1, 2
H264, HEVC = 0x1B, 0x24


class TsWriter:
    def __init__(self, out: BinaryIO) -> None:
        self.out = out
        self.carry = b""
        self.pmt_pids: set = set()
        self.kinds: Dict[int, int] = {}  # pid -> VIDEO
        self.t0: Optional[int] = None
        self.seconds = 0.0

    def feed(self, data: bytes) -> float:
        """Returns seconds of video received so far."""
        self.out.write(data)
        self.out.flush()
        buf = self.carry + data
        i = 0
        while i + 188 <= len(buf):
            if buf[i] != 0x47:
                i = buf.find(b"\x47", i + 1)
                if i < 0:
                    i = len(buf)
                continue
            self._packet(buf[i:i + 188])
            i += 188
        self.carry = buf[i:]
        return self.seconds

    def _packet(self, p: bytes) -> None:
        pid, start = (p[1] & 0x1F) << 8 | p[2], p[1] & 0x40
        if not p[3] & 0x10:
            return
        body = p[4 + (1 + p[4] if p[3] & 0x20 else 0):]
        if pid == 0 and start:
            sec = body[1 + body[0]:]
            for j in range(8, 3 + ((sec[1] & 0x0F) << 8 | sec[2]) - 4, 4):
                if sec[j] << 8 | sec[j + 1]:
                    self.pmt_pids.add((sec[j + 2] & 0x1F) << 8 | sec[j + 3])
        elif pid in self.pmt_pids and start and not self.kinds:
            sec = body[1 + body[0]:]
            end = 3 + ((sec[1] & 0x0F) << 8 | sec[2]) - 4
            j = 12 + ((sec[10] & 0x0F) << 8 | sec[11])
            while j + 5 <= end:
                kind, epid = sec[j], (sec[j + 1] & 0x1F) << 8 | sec[j + 2]
                if kind in (H264, HEVC):
                    self.kinds[epid] = VIDEO
                j += 5 + ((sec[j + 3] & 0x0F) << 8 | sec[j + 4])
        elif pid in self.kinds and start and len(body) >= 14 and body[7] & 0x80:
            b = body[9:14]
            pts = ((b[0] >> 1) & 7) << 30 | b[1] << 22 | (b[2] >> 1) << 15 | b[3] << 7 | b[4] >> 1
            self.t0 = pts if self.t0 is None else self.t0
            self.seconds = max(self.seconds, ((pts - self.t0) % (1 << 33)) / 90000)


def selfcheck() -> None:
    import io

    def pkt(pid: int, payload: bytes) -> bytes:
        return (bytes([0x47, 0x40 | pid >> 8, pid & 0xFF, 0x10]) + payload).ljust(188, b"\xff")

    def pes(pts: int) -> bytes:
        p = bytes([0x21 | (pts >> 29) & 0x0E, (pts >> 22) & 0xFF, (pts >> 14) & 0xFE | 1, (pts >> 7) & 0xFF, (pts << 1) & 0xFE | 1])
        return pkt(0x101, b"\x00\x00\x01\xe0\x00\x00\x80\x80\x05" + p)

    pat = pkt(0, bytes([0, 0, 0xB0, 13, 0, 1, 0xC1, 0, 0, 0, 1, 0xE1, 0]) + b"crc!")
    pmt = pkt(0x100, bytes([0, 2, 0xB0, 18, 0, 1, 0xC1, 0, 0, 0xE1, 1, 0xF0, 0, HEVC, 0xE1, 1, 0xF0, 0]) + b"crc!")
    data = pat + pmt + pes(900000) + pes(900000 + 90000 * 7)
    w = TsWriter(io.BytesIO())
    w.feed(data[:300]), w.feed(data[300:])
    assert w.seconds == 7 and w.out.getvalue() == data
