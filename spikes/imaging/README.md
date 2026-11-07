# Spike: Rust large-image processing (throwaway)

Benchmark crate for `spike/rust-imaging` (#22). **Not production code.** It is a standalone cargo crate, not part of any workspace.
Findings: [docs/research/rust-imaging.md](../../docs/research/rust-imaging.md). Proposal: [proposed-large-image-strategy.md](../../docs/architecture/adr/proposed-large-image-strategy.md).

## What it does

| Subcommand | Measures |
|---|---|
| `gen` | Generates a procedural battlemap (rooms, flagstones, walls, props, 100 px grid) and writes PNG, JPEG q90 and WebP q90 |
| `decode` | Decode time per decoder (`image` with and without its default limits, `png` row streaming, libwebp) |
| `pyramid` | 512 px tile pyramid to a disk cache, per tile format, full decode vs PNG row streaming |
| `region` | Region + context extraction, downscale to the provider's max input, upscale back; filter speed and quality |
| `blend` | Feathered-mask patch blend, colour match, seam metrics, proof crops, sRGB vs linear parity |
| `flatten` | Base + N transformed patch layers composed in strips with tiny-skia and streamed into PNG/JPEG; WebP needs the full image |
| `limits` | Encoder dimension limits (WebP 16383/16384, JPEG 65535, animated WebP, GIF) |
| `determinism` | `fast_image_resize` output across SIMD back-ends; blend repeatability |
| `anim` | Animated WebP (libwebp `WebPAnimEncoder`, pure-Rust lossless mux) and GIF (gif + NeuQuant) |
| `compare` | Pixel diff of two RGB PNGs (strip-height invariance check) |
| `animcheck` | Decodes an animated WebP with libwebp's `WebPAnimDecoder` and counts frames |

Every case runs in its own process and prints `RESULT` rows plus a `PEAK` row (peak working set and peak commit, Windows; VmHWM on Linux).

## Run

Needs Rust ≥ 1.86 and a C compiler. A cold release build takes ~4 min (libwebp is compiled from vendored source by `libwebp-sys`: MSVC on Windows, Xcode CLT on macOS).
The full run needs about 4 GiB free RAM and 3 GiB of disk. Generated images go to `$PF_DATA` (default `$TEMP/pocket-forge/imaging`), never into the repo.

```bash
cd spikes/imaging
bash run.sh                 # everything (~25 min on a 24-core desktop; PNG Balanced encodes dominate)
bash run.sh gen decode      # selected sections
target/release/pf-imaging-spike blend --proof /some/dir/seam.png   # one case
```

Results are appended to `$PF_DATA/results.tsv`. The proof image in the research doc comes from `blend`.
