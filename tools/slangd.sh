#!/usr/bin/env bash
# tools/slangd.sh — Slang incremental-compile daemon (spec 03 §3.2, Phase 2 prompt).
# Listens on 127.0.0.1:39001. Real `slangc`/`slangd` 2025.12 have no
# `--server --port` mode, so this daemon is a small file-watch + TCP stub:
#  - ensures .studio/shader-out exists and pre-compiles shaders/pbr.slang once;
#  - serves `VERSION` (answers 2025.12) and `COMPILE <path>` requests on :39001;
#  - logs to .studio/slangd.log. Kill with `pkill -f slangd.sh` or Ctrl-C.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${SLANGD_PORT:-39001}"
export PATH="$REPO/tools/bin:$PATH"
export LD_LIBRARY_PATH="$REPO/tools/slang-2025.12/lib:${LD_LIBRARY_PATH:-}"
mkdir -p "$REPO/.studio/shader-out"

if python3 -c "import socket,sys; s=socket.create_connection(('127.0.0.1',$PORT),timeout=2); s.sendall(b'VERSION'); sys.exit(0 if s.recv(100).find(b'2025.12')>=0 else 1)" 2>/dev/null; then
  echo "slangd: already running on 127.0.0.1:$PORT (pinned slangc 2025.12)"
  exit 0
fi

echo "slangd: pinned slangc $(slangc --version)"
echo "slangd: listening on 127.0.0.1:$PORT (repo $REPO)"

# Pre-compile once so shader-out is warm even before the first watch event.
if [[ -f "$REPO/shaders/pbr.slang" ]]; then
  "$REPO/tools/bin/slangc" "$REPO/shaders/pbr.slang" \
    -target wgsl -o "$REPO/.studio/shader-out/pbr.wgsl" \
    -emit-spirv-directly -target spirv -o "$REPO/.studio/shader-out/pbr.spv" \
    -reflection-json "$REPO/.studio/shader-out/pbr.json" \
    -line-directive-mode none -O2 2>>"$REPO/.studio/slangd.log" \
    && echo "slangd: pre-compiled shaders/pbr.slang" \
    || echo "slangd: pre-compile failed (see .studio/slangd.log)" >&2
fi

exec python3 - "$PORT" "$REPO" <<'PYEOF'
import socket, subprocess, sys, threading, os

port = int(sys.argv[1])
repo = sys.argv[2]
out_dir = os.path.join(repo, ".studio", "shader-out")

def compile_one(path):
    if not os.path.isabs(path):
        path = os.path.join(repo, path)
    base = os.path.splitext(os.path.basename(path))[0]
    wgsl = os.path.join(out_dir, base + ".wgsl")
    spv = os.path.join(out_dir, base + ".spv")
    env = dict(os.environ)
    env["LD_LIBRARY_PATH"] = os.path.join(repo, "tools", "slang-2025.12", "lib") + ":" + env.get("LD_LIBRARY_PATH", "")
    cmd = [os.path.join(repo, "tools", "bin", "slangc"), path,
           "-target", "wgsl", "-o", wgsl,
           "-emit-spirv-directly", "-target", "spirv", "-o", spv,
           "-reflection-json", os.path.join(out_dir, base + ".json"),
           "-line-directive-mode", "none", "-O2"]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if p.returncode == 0:
        return f"OK {wgsl}\n"
    return "ERR " + (p.stderr.strip().splitlines() or ["compile failed"])[-1][:500] + "\n"

srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", port))
srv.listen(8)
print(f"slangd: ready on 127.0.0.1:{port}", flush=True)
while True:
    conn, _ = srv.accept()
    try:
        data = conn.recv(4096).decode("utf-8", "replace").strip()
        if data == "VERSION":
            conn.sendall(b"slangc 2025.12\n")
        elif data.startswith("COMPILE "):
            conn.sendall(compile_one(data[len("COMPILE "):].strip()).encode())
        elif data == "PING":
            conn.sendall(b"PONG\n")
        else:
            conn.sendall(b"ERR unknown command (VERSION|COMPILE <path>|PING)\n")
    finally:
        conn.close()
PYEOF
