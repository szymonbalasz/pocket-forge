#!/usr/bin/env bash
# Runs the whole spike benchmark, one process per case (so each peak-memory
# figure belongs to one case). Heavy: needs ~4 GiB free RAM and ~3 GiB disk.
# Usage: bash run.sh [section...]   sections: gen decode pyramid region blend flatten limits determinism anim
# Output: RESULT/PEAK lines appended to $DATA/results.tsv. Generated images stay in $DATA (outside the repo).
set -euo pipefail
cd "$(dirname "$0")"
DATA="${PF_DATA:-${TMPDIR:-${TEMP:-/tmp}}/pocket-forge/imaging}"
mkdir -p "$DATA"
BIN=target/release/pf-imaging-spike
cargo build --release --quiet
OUT="$DATA/results.tsv"
run() { echo "+ $*" >&2; "$BIN" "$@" | tee -a "$OUT" | grep -E '^(RESULT|PEAK)' >&2 || true; }
sections=("$@")
[ ${#sections[@]} -eq 0 ] && sections=(gen decode pyramid region blend flatten limits determinism anim)

for s in "${sections[@]}"; do
  case "$s" in
  gen)
    run gen --size 8192 --dir "$DATA"
    run gen --size 16384 --dir "$DATA" ;;
  decode)
    for n in 8192 16384; do
      run decode --file "$DATA/map-$n.png" --with image-default
      run decode --file "$DATA/map-$n.png" --with image
      run decode --file "$DATA/map-$n.png" --with image-rgba
      run decode --file "$DATA/map-$n.png" --with png-rows
      run decode --file "$DATA/map-$n.jpg" --with image
      run decode --file "$DATA/map-$n.webp" --with image
      run decode --file "$DATA/map-$n.webp" --with libwebp
    done ;;
  pyramid)
    for f in jpeg webp webp-ll png-fast; do
      run pyramid --file "$DATA/map-16384.png" --fmt "$f" --tile 512 --out "$DATA/tiles" --mode full
    done
    run pyramid --file "$DATA/map-16384.png" --fmt jpeg --tile 256 --out "$DATA/tiles" --mode full
    run pyramid --file "$DATA/map-16384.png" --fmt jpeg --tile 512 --out "$DATA/tiles" --mode stream
    run pyramid --file "$DATA/map-16384.png" --fmt webp --tile 512 --out "$DATA/tiles" --mode stream ;;
  region)
    run region --size 16384 ;;
  blend)
    run blend --proof "$DATA/seam-proof.png" ;;
  flatten)
    # Memory strategy: strip vs one full-height strip, base streamed from the 16k PNG.
    run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt png-fast --base "$DATA/map-16384.png" --out "$DATA/flat-strip.png"
    run flatten --size 16384 --layers 8 --strip 16384 --par 1 --fmt png-fast --base "$DATA/map-16384.png" --out "$DATA/flat-full.png"
    run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt png --base "$DATA/map-16384.png" --out "$DATA/flat-strip-bal.png"
    run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt jpeg --base "$DATA/map-16384.png" --out "$DATA/flat-strip.jpg"
    run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt webp --base "$DATA/map-16384.png" --out "$DATA/flat-16k.webp"
    # WebP at its maximum-friendly size (10k) and the same 10k for PNG/JPEG
    for f in webp webp-ll png-fast jpeg; do
      run flatten --size 10240 --layers 8 --strip 512 --par 8 --fmt "$f" --base synth --out "$DATA/flat-10k.$f"
    done
    # Determinism: one thread vs all threads, compose-only
    RAYON_NUM_THREADS=1 run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt none --base "$DATA/map-16384.png"
    run flatten --size 16384 --layers 8 --strip 512 --par 8 --fmt none --base "$DATA/map-16384.png"
    run flatten --size 16384 --layers 8 --strip 256 --par 4 --fmt none --base "$DATA/map-16384.png"
    # Strip-height invariance: strip 512 vs one full-height strip
    run compare --a "$DATA/flat-strip.png" --b "$DATA/flat-full.png" ;;
  limits)
    run limits ;;
  determinism)
    run determinism ;;
  anim)
    run anim --clip token --out "$DATA/anim"
    run anim --clip map --out "$DATA/anim"
    for f in token-webp-pure-rust-lossless map-webp-pure-rust-lossless map-webp-libwebp-lossy-q75; do
      run animcheck --file "$DATA/anim/$f.webp"
    done ;;
  *) echo "unknown section $s" >&2; exit 2 ;;
  esac
done
echo "done: $OUT" >&2
