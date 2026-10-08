"""Record an mpv IPC transcript for the attribution test.

Starts its own silent `mpv --idle --ao=null`, observes the properties wavelet
observes, loads a 1 s sine, replaces it with another before it ends, lets that
one reach eof, then loads a missing file to capture an error. Writes every line
mpv sends, one JSON object per line.

usage: python3 -I tools/record_mpv.py fixtures/mpv_transcript.jsonl
"""

import json, os, socket, subprocess, sys, time

out_path = sys.argv[1]
run = os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp"), "wavelet-record")
os.makedirs(run, exist_ok=True)
sock_path = os.path.join(run, "mpv.sock")
if os.path.exists(sock_path):
    os.remove(sock_path)
mpv = subprocess.Popen(
    ["mpv", "--idle=yes", "--no-video", "--no-config", "--no-terminal", "--ao=null",
     f"--input-ipc-server={sock_path}"],
    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    for _ in range(250):
        try:
            s = socket.socket(socket.AF_UNIX)
            s.connect(sock_path)
            break
        except OSError:
            time.sleep(0.02)
    s.settimeout(0.05)
    lines, buf = [], b""

    def send(cmd, rid=None):
        msg = {"command": cmd}
        if rid is not None:
            msg["request_id"] = rid
        s.sendall((json.dumps(msg) + "\n").encode())

    def pump(seconds):
        global buf
        end = time.time() + seconds
        while time.time() < end:
            try:
                chunk = s.recv(65536)
            except socket.timeout:
                continue
            if not chunk:
                return
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                if line.strip():
                    lines.append(line.decode())

    for i, p in enumerate(["time-pos", "duration", "metadata/by-key/icy-title"], start=1):
        send(["observe_property", i, p])
    pump(0.2)
    send(["loadfile", "av://lavfi:sine=f=440:d=1", "replace"], 101)
    pump(0.5)
    send(["loadfile", "av://lavfi:sine=f=660:d=1", "replace"], 102)
    pump(2.0)
    send(["loadfile", "/nonexistent/wavelet-missing.ogg", "replace"], 103)
    pump(1.0)
    with open(out_path, "w") as f:
        f.write("\n".join(lines) + "\n")
    print(f"{len(lines)} lines -> {out_path}")
finally:
    mpv.terminate()
    mpv.wait()
