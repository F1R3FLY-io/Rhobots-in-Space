import socket, base64, os, struct, json, time, hashlib

def _mask(p):
    m = os.urandom(4)
    return bytes(b ^ m[i % 4] for i, b in enumerate(p)), m

class WS:
    def __init__(self, host, port, path, timeout=5):
        self.s = socket.create_connection((host, port)); self.s.settimeout(timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall(f"GET {path} HTTP/1.1\r\nHost: h\r\nUpgrade: websocket\r\n"
                       f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                       f"Sec-WebSocket-Version: 13\r\n\r\n".encode())
        buf = b""
        while not buf.endswith(b"\r\n\r\n"): buf += self.s.recv(1)
        assert b"101" in buf.split(b"\r\n")[0], buf
        exp = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        assert exp.encode() in buf, "accept mismatch -- handshake is wrong"
        self.buf = []
    def send(self, text):
        p = text.encode(); mp, m = _mask(p)
        h = b"\x81"
        h += bytes([0x80 | len(p)]) if len(p) < 126 else bytes([0x80 | 126]) + struct.pack(">H", len(p))
        self.s.sendall(h + m + mp)
    def _read(self, n):
        out = b""
        while len(out) < n:
            c = self.s.recv(n - len(out))
            if not c: raise EOFError("closed")
            out += c
        return out
    def recv(self):
        h = self._read(2); ln = h[1] & 0x7f
        if ln == 126: ln = struct.unpack(">H", self._read(2))[0]
        elif ln == 127: ln = struct.unpack(">Q", self._read(8))[0]
        return json.loads(self._read(ln).decode())
    def until(self, tag, secs=8):
        for i, f in enumerate(self.buf):
            if f.get("t") == tag: return self.buf.pop(i)
        end = time.time() + secs
        while time.time() < end:
            f = self.recv()
            if f.get("t") == tag: return f
            self.buf.append(f)
        raise TimeoutError(f"no `{tag}` frame in {secs}s")
    def sees(self, tag, secs):
        try:
            self.until(tag, secs); return True
        except (TimeoutError, socket.timeout): return False
