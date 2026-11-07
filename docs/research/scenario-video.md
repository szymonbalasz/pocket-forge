# Research: Scenario animation and video capabilities

> Spike `spike/scenario-video`. Public sources only, checked 2026-11-07. No credentials were used and no credits were spent.
> This extends the [Scenario API research](scenario-api.md) and [ADR 0001](../architecture/adr/0001-provider-integration.md) (#14). Every claim links to a source in [Sources](#sources). Anything not confirmed is marked `unverified`.
> Animation is tier A, after the first milestone, tokens first (#27). The remaining OQ-ANI items are decided when the animated-tokens spec is written.

## Summary

**Pick: C (both).** The provider makes short clips. The app composites them over static art and encodes the animated WebP itself.

| Option | What it means | For | Against |
|---|---|---|---|
| **A. Provider video → app encodes animated WebP** | Run an image-to-video job, then optionally video background removal. Get the frames back and encode them in the app. | Uses Scenario's ~220 video models as-is. Works for whole tokens and item art. | Clip resolution is ≤ 1080p–4K, far below battlemap size (#11). Loops are rarely native. No alpha from any generator. |
| **B. In-app compositing of looped layers** | Keep the static asset. Add small looping layers (water, torches, a token aura) and encode the animated export in the app. | The unchanged pixels stay identical. Fits patch layers and regions. Works at full map size. | The app needs layer timing and loop blending. The motion itself still has to come from somewhere (A or procedural). |
| **C. Both** | A makes clips of a **region** (maps) or a whole token. B places them as looped layers and exports. | Matches the existing region → patch layer model. One export path for all asset types. | It's the most work. Needs a loop-seam strategy and an alpha strategy. |

Key findings:

- **Video runs on the same route as images.** It's `POST /generate/custom/{modelId}`, with the same async job, polling, `metadata.assetIds` and `dryRun` query parameter [V2][V23]. The ADR 0001 adapter fits. It only needs a `generate_clip`-style operation.
- **No model outputs animated WebP.** Video assets are `video/mp4`, `video/quicktime`, `video/webm` or `video/x-image-sequence+zip` [V23]. Scenario's own utilities convert video to GIF (`model_scenario-image-seq-to-video`, `model_scenario-resize-video`, `model_scenario-compose-video`) and video to frames (`model_scenario-video-to-image-seq`) [V3]. So **the app encodes animated WebP itself**, but it may not need a video decoder (see [Encoding](#output-formats-and-encoding)).
- **Seamless loops are rare.** Only Luma Ray 3.2 has a `loop` flag [V4]. The LTX‑2.5 Cinemagraph models "animate an image into a smoothly looping cinemagraph", but the loop is driven by the prompt and isn't guaranteed [V10][V28]. Everything else needs the first frame = last frame trick (`unverified`) or a ping-pong loop [V3][V16].
- **No generator outputs alpha.** Video background removal does. Pixelcut returns transparent WebM VP9 by default [V11]. BiRefNet v2 returns VP9 WebM or ProRes 4444 with alpha [V16]. LTX‑2.5 Alpha Gen returns a grayscale matte [V10]. Each is one job per clip, not one per frame.
- **No video model takes a `mask`.** The only region control is the Cinemagraph prompt, where a named element moves and the rest stays still [V10][V28]. A help article mentions masking "to guide the movement" without naming a model (`unverified`) [V26]. So a map region gets animated by **region crop → clip → in-app masked composite**.
- **No CU prices are published for video.** Cost scales with duration, resolution, audio and model tier [V25]. Some models bill per second [V5]. `dryRun` exists on the route, but whether it gives a real figure for video models is `unverified` [V23][V24].

## Model table (motion sources)

These are the image-to-video models that fit tokens and map elements. Avatar, lip-sync, audio and editing models are omitted. "—" = not documented. fps is rarely documented, but asset metadata carries `frameRate` and `duration` [V23].

| Model (id) | Start / end frame | Duration | fps | Resolution | Aspect | Seed | Loop | Src |
|---|---|---|---|---|---|---|---|---|
| Luma Ray 3.2 (`model_luma-ray-3-2`) | `startFrame` / `endFrame` | 5 s, 10 s (10 s not with loop) | — | 540p, 720p, 1080p | 9:16 → 21:9, incl. 1:1 | — | **`loop: true`** | [V4] |
| LTX‑2.5 Distilled / Full Cinemagraph (`model_ltx-2-5-distilled-cinemagraph`, `…-full-cinemagraph`) | `image` (required) | 9–121 frames, steps of 8 (default 25) | 24 | follows image, fits 1920² | follows image | yes | **prompt-driven** (`CINEMAGRAPH_MOTION` trigger) | [V10][V28] |
| Kling V3 I2V Pro / Standard (`model_kling-v3-i2v-pro`, `…-standard`) | `startImage` / `endImage` | 3–15 s | — | — | 9:16, 1:1, 16:9 | — | first = last (`unverified`) | [V5] |
| Kling O1 I2V (`model_kling-o1-i2v`) | `image` / `lastFrameImage` | 5, 10 s | — | — | — | — | first = last (`unverified`) | [V5] |
| Veo 3.1 / Fast / Lite (`model_veo3-1`, `…-fast`, `…-lite`) | `image` / `lastFrameImage` | 4, 6, 8 s | — | 720p, 1080p | 16:9, 9:16 | yes | first = last (`unverified`) | [V6] |
| Wan 2.2 I2V (`model_wan-2-2-i2v-a14b`) | `image` / `lastFrameImage` | 81–100 frames | 5–24 (default 16) | 480p, 720p | — | yes | first = last (`unverified`) | [V9] |
| Wan 2.7 I2V (`model_wan-2-7-i2v`) | `image` / `endImage` | 2–15 s | — | 720p, 1080p | — | yes | first = last (`unverified`) | [V9] |
| MiniMax H3 Max I2V (`model_minimax-h3-max-i2v`) | first (required) / last | 5–15 s | — | 480P, 768P | follows first frame | yes | first = last (`unverified`) | [V8] |
| FLUX.3 First-Last Frame (`model_bfl-flux-3-first-last-frame`) | `image` + `endImage` (both required) | 5–20 s | — | 720p, 1080p | auto, 21:9 → 9:16, incl. 1:1 | — | first = last (`unverified`) | [V17] |
| Pruna P-Video 2 (`model_pruna-p-video-2`) | `image` / `lastFrameImage` | 1–20 s | 24, 48 | 720p, 1080p | ignored with image | yes | first = last (`unverified`) | [V19] |
| Vidu Q3 Turbo I2V (`model_vidu-i2v-q3-turbo`) | 1 or 2 images | 1–16 s | — | 540p–1080p | — | yes | first = last (`unverified`) | [V13] |
| PixVerse V5 (`model_pixverse-v5`) | first / last | 5, 8 s | — | 360p–1080p | 16:9, 9:16, 1:1 | yes | first = last (`unverified`) | [V14] |
| Runway Gen4.5 (`model_runway-gen4-5`) | `image` (start) | 2–10 s | — | — | 7 presets incl. 960:960 | yes | — | [V7] |
| Seedance 2.5 (`model_bytedance-seedance-2-5`) | first / last | 4–30 s | — | 480p–1080p | 7 incl. 1:1 | — | — | [V12] |

There are also 30 Scenario utility models, plus the Pixelcut and Academia models, that the pipeline would chain [V3][V11][V16]:

| Utility (id) | Use | Key facts |
|---|---|---|
| Video to Image Sequence (`model_scenario-video-to-image-seq`) | Frames back to the app | `extractAllFrames` or `frameInterval`. The image format isn't stated. |
| Image Sequence to Video (`model_scenario-image-seq-to-video`) | GIF made on the provider side | ≤ 1,000 images → `mp4` / `gif`, fps 1–120, `loopCount` (0 = infinite for GIF), `pingpong` |
| Compose Video (`model_scenario-compose-video`) | Layer compositing on the provider side | Output mp4/mov/webm/gif, transparent background, ≤ 7680×4320. Layer fields are undocumented. |
| Reverse Video (`model_reverse-video`) | Ping-pong loop material | Takes a video |
| Pixelcut Video Background Removal (`model_pixelcut-video-background-removal`) | Alpha for tokens | "Transparent preserves alpha". Formats: `webm_vp9` (default), `mov_proresks`, `mkv_vp9`, `gif`. |
| BiRefNet v2 video (`model_birefnet-v2-video`) | Alpha / mask | Alpha in VP9 WebM and ProRes 4444. MP4 has no alpha. `outputMask`. |
| LTX‑2.5 Alpha Gen (`model_ltx-2-5-alpha-gen`) | Matte only | Grayscale matte. Input ≤ ~6.33 s at 24 fps. |
| Video Layers Extractor (`model_scenario-video-layers-extractor`) | Split clip into layers | "each saved with transparency". `maxLayers` 1–10. |

## Output formats and encoding

- **What comes back:** MP4 (most generators; Seedance also offers MOV), WebM/MOV/MKV for alpha, or GIF from the utilities. Runway Gen4.5 also offers `png_sequence` [V7][V11][V12][V23].
- **GIF can be made entirely on Scenario's side.** It has 1-bit transparency only ("limited transparency") [V16], so it's poor for anti-aliased tokens.
- **Animated WebP has to be encoded by the app.** Paths:

| Path | New dependency | Licence | Notes |
|---|---|---|---|
| **1. Provider extracts frames → app encodes** | `webp-animation` (wraps libwebp) | MIT/Apache wrapper [V30], libwebp BSD‑3 [V29] | No video decoder in the app. Costs an extra utility job per clip. Whether frames keep alpha is `unverified`. |
| 1b. Same, pure Rust | `image-webp` | MIT/Apache [V31] | Animated encoding is `unverified` [V31] |
| 2. Webview decodes (WebCodecs) | a JS demuxer library | — | WebCodecs needs a separate demuxer [V36]. VP9-alpha and macOS parity are `unverified`. |
| 3. Bundled FFmpeg | FFmpeg DLLs | LGPL 2.1+. GPL if built with GPL parts (e.g. libx264) [V33] | LGPL needs dynamic linking, shipping the matching source and credits [V33]. MPEG patent caveat [V33]. Heavy. |
| 4. Rust H.264 decode | `openh264` | BSD‑2 [V34] | H.264 only (no VP9 alpha). Patent risk for source builds: Cisco covers only its own downloaded binary [V35]. |

- **GIF export in the app (if wanted):** the `gif` crate is MIT/Apache [V32].
- **Pick: path 1.** It's MIT-compatible with no video codec, and §3.5 still applies to the new crate.

## Transparency (animated tokens)

| Option | Cost shape | Quality | Notes |
|---|---|---|---|
| **a. Video background removal job** (Pixelcut / BiRefNet v2) | +1 job per clip (CU `unverified`) | Temporal consistency is claimed [V11] | Output is VP9/ProRes with alpha. It then needs path 1 (alpha `unverified`) or an in-app VP9 decode. |
| b. Image background removal per frame | × frames (25–121 per clip) [V10] | Can flicker between frames (`unverified`) | Uses the existing image models [scenario-api.md](scenario-api.md#capability-matrix). Probably the most expensive. |
| c. Chroma key in the app | Free | Colour spill and fringes, and motion blur breaks the key (`unverified`) | Start the frame on a flat key colour. Models may drift the background. |
| d. Alpha Gen matte + RGB clip | +1 job | Soft matte | The app aligns two videos. ≤ ~6.33 s [V10]. |
| e. Static token + animated effect layers only (option B) | Free / per effect | The token art stays crisp | Avoids alpha video entirely. Limited to auras, rings and glows. |

**Pick: a, with e as the fallback.**

## Cost and time

- **Pricing:** no CU figures for any video model are published on the docs, help or pricing pages [V2][V24][V25][V27]. Stated cost drivers are duration, resolution, native audio, multi-shot and model tier. I2V "can cost more" than T2V [V25]. Kling 4K is "per second of video", and Kling 2.5 / Pika 10 s cost 2× of 5 s [V5][V15]. Images are ~2–15 CU each, for scale [V24].
- **CU to money:** the pricing page lists Starter 1,500 credits for $15 (≈ $0.01 each). The help pages disagree on Pro (5,000 vs 10,000) [V24][V27].
- **Dry run:** `dryRun` is a parameter of `/generate/custom/{modelId}` (response 269) [V23]. Help says it returns "the exact CU cost" [V24]. Video coverage is `unverified`. **The estimate must be re-run whenever duration, resolution or audio changes.**
- **Audio:** many models default to `generateAudio: true` [V5][V10][V17]. The app should always send `false`. Only BFL says audio is free [V17].
- **Job duration:** not documented anywhere (`unverified`). Clips are "typically 5-12 seconds" [V26]. Expect jobs to run past the 3-minute "still running" point in OQ-PRV-05. A chained pipeline (generate → remove background → extract frames) is three jobs.

## Region-only animation (maps)

- **No video model on Scenario lists a `mask` or motion-brush input** [V4]–[V22]. The retake models' "region" is a time window [V10].
- **The Cinemagraph models are the nearest thing.** "only specific elements move (water flowing…)", while "keeping backgrounds … completely still". Only one element should move, and camera motion is out of scope [V28].
- **Map size forces region work anyway.** Maps are 5–16k px (#11), and clips max out at 1080p–4K [V4][V5][V10].
- **Workable flow:** region (existing concept) → cinemagraph clip → in-app feathered **mask** so only the painted pixels animate → looped patch-layer-like element → animated export. This is option C.

## Model capabilities ("honest controls")

- **Same shape:** video models use the same `capabilities` enum (`txt2video`, `img2video`, `video2video`, `video2img`) and the same `inputs[]` schema. `kind: "video"` marks video file inputs [V23]. Docs pages show the same type/default/min/max/allowed-values tables [V3]–[V22].
- **Gaps for the overlay table (ADR 0001):**
  - **Frame-role names vary:** `image` / `startImage` / `startFrame` / `imageUrl`, and `lastFrameImage` / `endImage` / `endFrame` / `lastFrame` [V4][V5][V6][V9]. Map them like reference roles.
  - **Duration units vary:** seconds, `"5s"` strings and `numFrames` [V4][V9][V10]. Normalise to seconds.
  - **Loop support isn't a field.** It's either native (`loop`), prompt-driven or none. Record it in the overlay.
  - **Output container and alpha aren't model fields.** They appear only as per-model `outputFormat` inputs [V7][V11].
  - **Cross-field rules are written in prose:** "10s is unavailable with Loop", "audio not supported with last frame", "8s only with reference images" [V4][V5][V6]. Whether `required.conditional` encodes them is `unverified`.
- **The per-model LLM-markdown spec** (`app.scenario.com/api/models/{id}/markdown`) returned HTTP 429 to anonymous fetches [V37]. It's not needed, because `GET /models/{id}` is the source.

## Gaps and risks

- **Loop seams.** Only one model has a loop flag. The app may need a seam crossfade or ping-pong. Expect fixture tests on seam frames.
- **Model churn.** About 220 video models from 31 providers, with versioned IDs [V1][V2]. Keep the overlay small (2–3 curated models), and let capabilities drive the rest.
- **Job status naming differs.** The overview uses `processing` / `failed` / `failure`, while the OpenAPI has the canonical states [V2][V23]. Parse leniently, as ADR 0001 already does.
- **Fixtures.** Clip fixtures must be generated or CC0, never personal art (KQ-24, AGENTS §3.7).
- **A 3-job chain triples the failure points.** Each step needs its own estimate and cost, and a partial result has to be resumable.

## Unverified

- Whether first frame = last frame yields a seamless loop on any model.
- Whether Video to Image Sequence keeps alpha, and which image format it returns (`video/x-image-sequence+zip` is assumed).
- CU cost of any video or utility model. Whether `dryRun` returns a non-zero figure for video.
- Typical job duration per model. Any server-side timeout.
- Which models support the "masking to guide movement" the help article mentions [V26].
- Whether `image-webp` encodes animation. VP9-alpha decoding in WebView2 / WKWebView.
- Whether `required.conditional` captures the prose cross-field rules.
- How well Pixelcut's background removal holds up on fantasy tokens (needs one real call).

## Open questions

| ID | Question | Options (pick in **bold**) | Tradeoff |
|---|---|---|---|
| OQ-ANI-01 ✅ | Is animation in tier S? **Answered: no. Tier A, tokens first (#27).** | **A. No, tier A or later. Design later. This spike is input only.** / B. add it to S | A protects milestone 1. B adds a pipeline, an encoder and alpha handling to the MVP. |
| OQ-ANI-02 | Where does motion come from? | A. provider clip only / B. in-app layers only / **C. both: provider clips composited as looped layers** | C matches regions and patch layers and works at map size. It's the most work. |
| OQ-ANI-03 | How does the app get frames to encode? | **A. Provider frame extraction + `webp-animation` (libwebp, BSD)** / B. WebCodecs in the webview / C. bundled LGPL FFmpeg | A has no codec and an MIT-safe stack, but costs an extra job. C is heavy and has LGPL/patent obligations. |
| OQ-ANI-04 | Alpha for animated tokens? | **A. Video background removal job (Pixelcut), with static token + effect layers as the fallback** / B. chroma key in the app / C. per-frame background removal | A is one job with temporal consistency claimed. B is free but has fringes. C costs × frames. |
| OQ-ANI-05 | Map animation scope? | **A. Region only: crop → cinemagraph → masked composite** / B. whole-map clip | B is impossible at 5–16k px (#11) |
| OQ-ANI-06 | Export formats? | **A. Animated WebP only** / B. + GIF / C. + WebM | A is the owner's stated need. Add others after the target VTT limits are known (KQ-21). |
| OQ-ANI-07 | Glossary: a new term for a video job's output? | **A. "Clip": a short video a job returns, before it becomes a layer** / B. reuse "Candidate" | A keeps candidate = image. B blurs the existing comparison flow. |
| OQ-ANI-08 | Verify the `unverified` items with a real key? (This spends credits, so it's the owner's call.) | **A. The owner runs ~3 dry runs + 2 small real jobs (cinemagraph, Pixelcut → frames). Record the responses as fixtures.** / B. stay docs-only | A settles cost, alpha and loop quality for a few CU. B leaves the design guessing. |
| OQ-ANI-09 | Audio on video models? | **A. Always send `generateAudio: false`. Never offer audio.** / B. expose it | A avoids hidden cost. B has no use for VTT assets. |

Blocking for the animation build: **OQ-ANI-02, 08.**

## Sources

[V1]: https://docs.scenario.com/get-started/documentation/key-capabilities-at-a-glance
[V2]: https://docs.scenario.com/get-started/generation/video-generation
[V3]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-scenario
[V4]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-luma-labs
[V5]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-kuaishou-technology
[V6]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-google
[V7]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-runway-ml
[V8]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-minimax
[V9]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-alibaba
[V10]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-lightricks
[V11]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-pixelcut
[V12]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-bytedance
[V13]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-shengshu-technology
[V14]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-pixverse
[V15]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-pika
[V16]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-academia
[V17]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-bfl/
[V18]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-xai
[V19]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-pruna-ai/
[V20]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-openai
[V21]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-kandinsky-lab
[V22]: https://docs.scenario.com/get-started/generation/video-generation/video-generation-decart-ai
[V23]: https://cdn.cloud.scenario.com/static/api/openapi.json
[V24]: https://help.scenario.com/articles/7934059476-api-usage-and-credits-compute-units
[V25]: https://help.scenario.com/articles/8811829644-model-costs-for-asset-generation
[V26]: https://help.scenario.com/articles/4919258867-introduction-to-video-generation
[V27]: https://www.scenario.com/pricing
[V28]: https://huggingface.co/Lightricks/LTX-2.5-22b-LoRA-Cinemagraph
[V29]: https://chromium.googlesource.com/webm/libwebp/+/HEAD/COPYING
[V30]: https://crates.io/crates/webp-animation
[V31]: https://docs.rs/image-webp/latest/image_webp/
[V32]: https://crates.io/crates/gif
[V33]: https://ffmpeg.org/legal.html
[V34]: https://crates.io/crates/openh264
[V35]: https://www.openh264.org/BINARY_LICENSE.txt
[V36]: https://developer.mozilla.org/en-US/docs/Web/API/WebCodecs_API
[V37]: https://app.scenario.com/api/models/model_ltx-2-5-distilled-cinemagraph/markdown

| Ref | Source |
|---|---|
| V1 | [Key capabilities at a glance](https://docs.scenario.com/get-started/documentation/key-capabilities-at-a-glance) |
| V2 | [Video generation overview](https://docs.scenario.com/get-started/generation/video-generation) |
| V3 | [Scenario video models (utilities)](https://docs.scenario.com/get-started/generation/video-generation/video-generation-scenario) |
| V4 | [Luma Labs video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-luma-labs) |
| V5 | [Kling (Kuaishou) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-kuaishou-technology) |
| V6 | [Google video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-google) |
| V7 | [Runway video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-runway-ml) |
| V8 | [MiniMax video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-minimax) |
| V9 | [Alibaba (Wan) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-alibaba) |
| V10 | [Lightricks (LTX) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-lightricks) |
| V11 | [Pixelcut video background removal](https://docs.scenario.com/get-started/generation/video-generation/video-generation-pixelcut) |
| V12 | [Bytedance (Seedance) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-bytedance) |
| V13 | [Shengshu (Vidu) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-shengshu-technology) |
| V14 | [PixVerse video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-pixverse) |
| V15 | [Pika video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-pika) |
| V16 | [Academia video models (BiRefNet v2, Reverse Video)](https://docs.scenario.com/get-started/generation/video-generation/video-generation-academia) |
| V17 | [BFL (FLUX.3) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-bfl/) |
| V18 | [xAI video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-xai) |
| V19 | [Pruna AI video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-pruna-ai/) |
| V20 | [OpenAI (Sora) video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-openai) |
| V21 | [Kandinsky Lab video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-kandinsky-lab) |
| V22 | [Decart AI video models](https://docs.scenario.com/get-started/generation/video-generation/video-generation-decart-ai) |
| V23 | [Scenario REST API OpenAPI spec](https://cdn.cloud.scenario.com/static/api/openapi.json) (asset `mimeType` enum, `capabilities` enum, input `kind`, `frameRate`/`duration` metadata, `dryRun` on `/generate/custom/{modelId}`) |
| V24 | [API usage and Compute Units](https://help.scenario.com/articles/7934059476-api-usage-and-credits-compute-units) |
| V25 | [Model costs for asset generation](https://help.scenario.com/articles/8811829644-model-costs-for-asset-generation) |
| V26 | [Introduction to video generation](https://help.scenario.com/articles/4919258867-introduction-to-video-generation) |
| V27 | [Scenario pricing](https://www.scenario.com/pricing) |
| V28 | [LTX‑2.5 Cinemagraph LoRA model card](https://huggingface.co/Lightricks/LTX-2.5-22b-LoRA-Cinemagraph) |
| V29 | [libwebp licence](https://chromium.googlesource.com/webm/libwebp/+/HEAD/COPYING) |
| V30 | [`webp-animation` crate](https://crates.io/crates/webp-animation) |
| V31 | [`image-webp` crate docs](https://docs.rs/image-webp/latest/image_webp/) |
| V32 | [`gif` crate](https://crates.io/crates/gif) |
| V33 | [FFmpeg legal](https://ffmpeg.org/legal.html) |
| V34 | [`openh264` crate](https://crates.io/crates/openh264) |
| V35 | [OpenH264 binary licence](https://www.openh264.org/BINARY_LICENSE.txt) |
| V36 | [MDN: WebCodecs API](https://developer.mozilla.org/en-US/docs/Web/API/WebCodecs_API) |
| V37 | [Per-model markdown spec (example)](https://app.scenario.com/api/models/model_ltx-2-5-distilled-cinemagraph/markdown) (HTTP 429 to anonymous fetch on 2026-11-07) |
