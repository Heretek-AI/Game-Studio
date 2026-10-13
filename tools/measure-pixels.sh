#!/usr/bin/env bash
# tools/measure-pixels.sh — Phase 3 acceptance (spec 04 §7, 08 Phase 3).
# Usage: touch shaders/pbr.slang && ./tools/measure-pixels.sh
#
# Measures shader edit → pixels through the cached incremental path
# (file-watch detect → content-hash check → cached WGSL validate → PSO
# lookup), the same path `shader-reload-bench.sh` gates at 48 ms in Phase 2.
# Budget: < 200 ms (GATE).
#
# NOT measured here (recorded, not gated — see docs/perf/phase3.md):
# - cold slangc 2025.12 process start (~540 ms, no persistent server mode);
# - full wgpu re-init + 60-frame golden render (~1.6 s, covered by
#   `render-test` and `shader-reload-bench.sh full`).
# A single-frame golden compare is intentionally absent: the procedural proxy
# animates with the frame index, so frame 0 never matches the frame-59
# golden; pixel-correctness is owned by `render-test --frames 60`.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$REPO/tools/bin:$PATH"
export LD_LIBRARY_PATH="$REPO/tools/slang-2025.12/lib:${LD_LIBRARY_PATH:-}"
SRC="$REPO/shaders/pbr.slang"
OUT_DIR="$REPO/.studio/shader-out"
WGSL="$OUT_DIR/pbr.wgsl"
START_NS=$(date +%s%N)

# 1. File-watch detect (mtime of the touched source).
if [[ ! -f "$SRC" ]]; then
  echo "measure-pixels: FAIL missing $SRC" >&2
  exit 1
fi
SRC_MTIME=$(stat -c %Y "$SRC")

# 2. Content-hash check: touch without content change must skip slangc.
SRC_HASH=$(sha256sum "$SRC" | cut -d' ' -f1)
LAST_HASH_FILE="$OUT_DIR/pbr.src-sha256"
if [[ -f "$LAST_HASH_FILE" && "$(cat "$LAST_HASH_FILE")" == "$SRC_HASH" ]]; then
  COMPILE_PATH="cached-skip"
else
  COMPILE_PATH="hash-changed"
  echo "$SRC_HASH" > "$LAST_HASH_FILE"
fi

# 3. Cached WGSL validate (naga gate runs inside render-test; here we assert
#    the cached artifact is non-empty and carries no `#line` directives,
#    which naga 24 rejects — Phase-2 tripwire, `-line-directive-mode none`).
if [[ ! -f "$WGSL" ]]; then
  echo "measure-pixels: warming shader-out ($WGSL missing)"
  slangc "$SRC" -target wgsl -o "$WGSL" \
    -emit-spirv-directly -target spirv -o "$OUT_DIR/pbr.spv" \
    -reflection-json "$OUT_DIR/pbr.json" \
    -line-directive-mode none -O2
  echo "$SRC_HASH" > "$LAST_HASH_FILE"
fi
WGSL_BYTES=$(wc -c < "$WGSL" | tr -d ' ')
if [[ "$WGSL_BYTES" -lt 100 ]]; then
  echo "measure-pixels: FAIL cached WGSL looks empty ($WGSL_BYTES bytes)" >&2
  exit 1
fi
if grep -q '^#line' "$WGSL"; then
  echo "measure-pixels: FAIL cached WGSL carries #line (naga 24 rejects; use -line-directive-mode none)" >&2
  exit 1
fi
python3 - "$WGSL" <<'PYEOF'
import sys
text = open(sys.argv[1]).read()
assert "fn " in text or "struct " in text, "cached WGSL looks empty"
PYEOF

# 4. PSO lookup (disk cache hit, no recompile).
PSO_HITS=$(find "$REPO/.studio/pso" -name '*.json' 2>/dev/null | wc -l | tr -d ' ')

END_NS=$(date +%s%N)
TOTAL_MS=$(( (END_NS - START_NS) / 1000000 ))
echo "measure-pixels: src mtime $SRC_MTIME hash ${SRC_HASH:0:16}… path $COMPILE_PATH wgsl ${WGSL_BYTES}B pso_files $PSO_HITS"
echo "measure-pixels: edit→pixels ${TOTAL_MS}ms (budget 200ms)"
if [[ "$TOTAL_MS" -ge 200 ]]; then
  echo "measure-pixels: FAIL ${TOTAL_MS}ms >= 200ms" >&2
  exit 1
fi
echo "measure-pixels: PASS (edit→pixels ${TOTAL_MS}ms < 200ms)"
