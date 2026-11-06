# Research: Scenario REST API vs MCP server vs TS SDK

> Spike `spike/scenario-api` for KQ-04. Public sources only, checked 2026-11-06. No credentials were used and no credits were spent.
> Every claim links to a source in [Sources](#sources). Anything not confirmed is listed under [Unverified](#unverified).

## Summary and recommendation

**Recommendation: build a hand-written REST adapter in the Rust core. Use neither MCP nor the TS SDK in the app.** MCP stays useful for agent-driven exploration outside the app. The SDK is a good reference implementation to read.

There are three surfaces, and all of them sit on one REST API:

- **REST** (`api.cloud.scenario.com/v1`) has a public OpenAPI 3.0 spec [S1].
- **TS SDK** `@scenario-labs/sdk` is "generated from our OpenAPI spec by Stainless", plus hand-written helpers [S31][S32].
- **MCP server** (`mcp.scenario.com/mcp`) is an agent layer over the same API [S10][S11].

Why REST from Rust:

- **REST has everything the milestone loop needs.** It covers upload, run (async job), job polling, cancel, a free cost estimate (`?dryRun=true` returns HTTP 269), per-job cost, and the remaining balance [S1][S2][S3][S4].
- **Model capabilities are machine-readable.** `GET /models/{modelId}` returns `inputs[]` with type, min/max, allowed values, `maxSize`, `maskFrom`, `costImpact` and conditional `required` rules [S1]. This makes "honest controls" possible.
- **MCP adds nothing the app needs and costs stability.** It is 0.x, shipped 10 releases in about 2 months, and has no stated stability promise [S10][S18]. Its extras (memory, `prompt_spark`, recommendations) are LLM-agent features, and #9 excludes those from the app. Images don't flow through it above ~100 KB anyway [S13].
- **The SDK would put the provider in the webview or in a Node sidecar.** In the webview, the secret lives in UI JavaScript (ToS §7.2 warns against keys in client-side apps) and CORS is unknown. A sidecar adds a bundled Node runtime and pushes against #3. It also churns: 3 majors in 7 months [S17][S31][S32].
- **Gaps to design around** (no blockers): mask polarity differs per model and is only stated in prose, webhooks need a public HTTPS endpoint (so the desktop app polls), signed-URL expiry is undocumented, and the capabilities schema itself drifts.

## Capability matrix

`CU` = Scenario's Compute Units (its credit unit). "Catalog" = an MCP tool hidden by default and reached through `scenario_tool_execute_*` [S10]. The SDK column names the typed method. Each SDK method is a 1:1 wrapper of the REST route unless noted [S31][S33].

| Capability | REST | MCP | TS SDK | Notes | Source |
|---|---|---|---|---|---|
| **Auth** | HTTP Basic: `base64(key:secret)`. The spec also declares a bearer JWT scheme. | Remote OAuth (recommended), or the same Basic key in a header | `apiKey`/`apiSecret`, or `bearerAuth` + `withBearerRefresh` | API access needs a paid plan. A key pair suits the OS credential store (KQ-05). MCP OAuth means a browser login. | [S1][S5][S10][S17][S31] |
| **Endpoint and transport** | `https://api.cloud.scenario.com/v1`, JSON | Streamable HTTP at `https://mcp.scenario.com/mcp` (stdio only via the `mcp-remote` bridge) | In-process JS calling REST via `fetch`. Browser, Node ≥ 20, Deno, Bun. | All three are hosted by Scenario | [S1][S10][S31] |
| **List models** | `GET /models` (filter by `type`, ≤ 500 per page), `POST /search/models`, `GET /recommendations/models` | `models_list`, `search`, `recommend` | `models.list()` (auto-pagination), `search.*` | Covers Scenario-trained, custom LoRA and third-party models | [S1][S10][S19][S31] |
| **Per-model parameters (honest controls)** | **Yes.** `GET /models/{id}` → `capabilities[]`, `inputs[]` (`type`, `min`/`max`, `step`, `allowedValues`, `default`, `maxSize`, `kind`, `maskFrom`, `costImpact`, `required.{always,ifDefined,ifNotDefined,conditional}`) | **Yes.** `model_schema_get` (reshaped: `allowed_values`, `cost_impact`) | `models.retrieve()`, typed `inputs` | Mask polarity and reference role are **not** fields. Schema drift: SDK 3.3.0 notes the API replaced `required.conditionalValues` with `required.conditional`. | [S1][S12][S32] |
| **Text-to-image** | `POST /generate/custom/{modelId}` | `model_run` | `generate.runModel()` → `job.wait()` | The body is per model and typed `unknown` everywhere. Legacy `/generate/txt2img`-style routes are deprecated. | [S1][S6][S11][S33] |
| **Image-to-image / strength** | Per model (`image`, `referenceImages`). `strength` only on the deprecated legacy routes. | Same via `model_run` | Same via `runModel()` | Most current models are instruction-edit models with no strength knob | [S1][S20] |
| **Inpainting (mask)** | Per model through a `mask` file input. GPT Image 2 / 2.5: PNG, **transparent alpha = edit**, same size as the first reference, ≤ 3840 px, ≤ 50 MB per image. Ideogram 4.5 / 4.5 Precise Edit: **white = edit** (Precise Edit takes inputs up to 67 MP and keeps the input size). Ideogram V3 / Character: **black = inpaint**. | Same via `model_run` | Same via `runModel()` | **Three mask conventions.** No BFL or Google model lists a `mask`. Legacy `/generate/inpaint` is deprecated. | [S1][S21][S22][S23][S24] |
| **Outpainting / extension** | Models: Smart Reframe (up to ~17 MP), Gemini Reframe | `model_run` | `runModel()` | — | [S20] |
| **Structure control** | FLUX.1 Dev `controlImage`/`controlModality`/`controlStrength`. Scenario Detection extracts canny/depth/pose/sketch maps. | `model_run`, `asset_detect` | `runModel()`, `generate.detect()` | A per-model input, not a generic API concept | [S20][S21][S10] |
| **Style reference** | `referenceImages` (FLUX.2 ≤ 5, Gemini ≤ 14, Ideogram V3 ≤ 4 style refs) | `model_run` | `runModel()` | One array mixes style and content. Reference roles aren't separated. | [S21][S23][S24] |
| **Character / identity reference** | Ideogram Character (character ref required). Trained subject LoRAs. | `model_run`, `train` | `runModel()`, `models.train.*` | — | [S23][S6] |
| **Upscaling** | Upscale models (Magnific, Topaz, Clarity, Recraft, BFL, …) | `model_run` | `runModel()` | Legacy `/generate/upscale` is deprecated | [S6][S25] |
| **Background removal (tokens)** | Models from Photoroom, Bria, Pixelcut, Ideogram | `model_run` | `runModel()` | Legacy route is deprecated | [S6][S26] |
| **Seamless / tiling** | Scenario Texture, Ideogram V4 Tiling, PATINA Material | `model_run` | `runModel()` | — | [S20][S23][S27] |
| **Seeds / reproducibility** | A `seed` input on many models (FLUX, Ideogram). **None listed** for GPT Image or Gemini. A job stores `metadata.input`. | Same | Same | Record model id + inputs + seed (KQ-17) | [S2][S21][S22][S24] |
| **Upload** | `POST /assets` (base64, < 4 MB). `POST /uploads` presigned multipart (≥ 5 MB parts, ≤ 10,000) + `complete`. Or a data URL directly in a file input. | `upload_asset` (inline ≤ ~100 KB, else presigned PUTs the client performs) + `upload_asset_complete` | `uploads.uploadFile()` does init + parts + complete + poll in one call | Formats: PNG, JPEG, WebP, AVIF, TIFF, HEIC, GIF, SVG. No max dimensions documented. | [S1][S7][S8][S13][S33] |
| **Result retrieval** | Job `metadata.assetIds` → `GET /assets/{id}` → `asset.url` ("Signed URL") | `jobs_wait` → `asset_download` (URL) | `assets.retrieve()` → `asset.download()` | URL expiry isn't documented | [S1][S9][S14][S33] |
| **Job lifecycle** | Async. `GET /jobs/{id}`: `queued, warming-up, pending, in-progress, finalizing, success, failure, canceled`, `progress` 0–1, `metadata.error`/`hint` | `model_run wait=true` blocks ≤ 180 s, then `jobs_wait` (≤ 32 jobs) | `job.wait()` polls every 3 s, 120 s default timeout | Docs example: poll 2 s, timeout 120 s | [S2][S15][S3][S32] |
| **Cancel** | `POST /jobs/{id}/action {action:"cancel"}` (inference only) | `job_cancel` | `jobs.triggerAction()` | — | [S1][S10][S31] |
| **Webhooks** | Project endpoints (≤ 10), HMAC-SHA256, 3 attempts, `generation.completed/failed`, `team.credit.threshold.crossed` | None | Webhook-endpoint CRUD | Needs a reachable HTTPS URL, so it doesn't fit a desktop app (#3) | [S16][S1] |
| **Errors / retries** | `reason` (+ `name`, `details`, `hint`). Gateway errors use `message`. Retry 429/5xx with backoff. | Tool error text | Typed `APIError` subclasses. Auto-retries 2× on connection errors, 408, 409, 429, ≥ 500. 60 s request timeout. | — | [S3][S31] |
| **Rate limits / concurrency** | 429. Per-plan `limits.parallelInferences`, `parallelCustomJobs`, `inferenceBatchSize` via `GET /teams/{id}/limits`. No numbers published. | Same backend | Same backend | ToS: limits vary by plan | [S1][S3][S17] |
| **Cost estimate (pre-flight)** | **Yes.** `?dryRun=true` → 269 with `creativeUnitsCost` and `costDetails`, "nothing is executed nor consumed" | `model_run dry_run=true` | `dryRun` query param | `ipDetection=true` charges even on a dry run. Never set it. | [S1][S4][S11] |
| **Cost per job** | 200 response: `creativeUnitsCost`, `costDetails`. Job: `billing.cuCost`, `cuDiscount`, `cuCostDetails`. | `jobs_wait` rows carry `cuCost` | Same fields, typed | Failed jobs are refunded unless `disableRefund` | [S1][S2][S15] |
| **Balance** | `GET /teams/{teamId}/limits` → `limits.creativeUnits` vs `values.creativeUnits`, plus `oneShotCreativeUnits[].remaining`. `GET /usages` for history. | `consumption_limit_get` (member/project cap, **not** the team plan balance), `usage` | `teams.retrieveLimits()`, `usages.list()` | Rough price: image gen ≈ 2–15 CU | [S1][S4][S28][S29][S31] |

## MCP server specifics

- **Transport and hosting:** remote Streamable HTTP, hosted by Scenario. There's no local server [S10].
- **Auth:** OAuth (Dynamic Client Registration), or a Basic key header. OAuth callers pass `team_id` and `project_id` on each call [S10][S11].
- **Tools:** 99 are stated and 97 are listed, in groups: Discovery, Generation (`model_schema_get`, `model_run`, `prompt_spark`, `train`), Analysis, Assets, Collections, Models, Workflows, Jobs (`job_get`, `jobs_wait`, `jobs_list`, `job_cancel`), Projects, Members, API Keys, Memory, Operations (`usage`, `consumption_limit_get`, …) and a Tool Catalog meta-layer. The default list is "lean". `?toolsets=full` shows all of them [S10].
- **Thin wrapper?** Mostly. `dry_run` is literally `?dryRun=true` [S11]. Its additions are agent conveniences: server-side waits with progress notifications, memory, prompt rewriting, skills, recommendations [S10][S15].
- **MCP-only:** `memory_*`, `prompt_spark`, `skill_get`, `plan_generation`, `asset_display`, server-side `jobs_wait` [S10].
- **REST/SDK-only:** webhooks, team plan balance, `/generate/patch`, `/oscu/prices` [S1].
- **Images and masks:** passed by **asset ID** only [S11]. Base64 is inline only up to ~100 KB. Larger files go through presigned PUTs [S13]. Outputs come back as download URLs [S14].
- **Maintenance:** very active (v0.34 → v0.38 in Oct–Nov 2026), still 0.x. Past releases include a "Toolset Redesign" (v0.16) and "Standardized Tool Namespaces" (v0.18). There's no version or deprecation policy [S10][S18].

## TS SDK specifics (`@scenario-labs/sdk`)

- **What it wraps:** the REST API only. The source header says "File generated from our OpenAPI spec by Stainless" [S31]. It adds hand-written helpers in `lib/`: `job.wait()`, `uploads.uploadFile()`, `asset.download()`, bearer refresh, and workflow conversion/validation using a CEL dependency [S31][S33].
- **Coverage:** the same resources as REST (assets, collections, generate, jobs, models, pricing, projects, search, teams, uploads, usages, workflows) [S31]. It has nothing MCP-only (no memory or prompt rewriting). The run body is untyped, as in REST [S1].
- **Maintenance:** MIT licence. 24 versions from 2026-04-14 to 2026-10-30, roughly every 1–3 weeks. Majors: 1.0.0 (Apr), 2.0.0 (May), 3.0.0 (Aug) [S32]. It "generally follows SemVer", but "certain backwards-incompatible changes may be released as minor versions" [S31]. The GitHub repo linked from npm returns 404 (not public) [S31].
- **Runtimes:** web browsers, Node ≥ 20, Deno, Bun and edge runtimes. A custom `fetch` can be injected [S31].
- **Usable from our stack?**
  - *In the webview:* technically yes, but the key and secret would sit in UI JavaScript. ToS §7.2 says not to embed keys in "client-side applications" [S17]. Whether Scenario's CORS allows the app's webview origin is unverified. It would split job state, cost and fixtures across the TS/Rust boundary. An injected Tauri HTTP `fetch` would avoid CORS but is a new dependency.
  - *Node sidecar:* bundles a Node runtime (≥ 20) and adds IPC. Over stdio it isn't a localhost server, but it is a second runtime to ship and update, which goes against the spirit of #3.
  - *Verdict:* the Rust REST client is simpler, keeps the secret in the core, and keeps one fake provider for KQ-24. **Read the SDK as a reference** for upload and polling behaviour and its typed shapes.

## Feasibility from our stack

| Route | Fit | Notes |
|---|---|---|
| **REST from Rust** | Good | Public OpenAPI 3.0.0 spec (~8 MB) [S1]. Codegen buys little because the run body is `unknown`. A hand-written client for ~12 endpoints is small. Uploads go straight to presigned URLs. Errors are HTTP + `reason`/`hint` [S3]. Fixtures are plain JSON pairs (KQ-24). ToS gives 90 days' notice for breaking API changes and 12 months to retire a version [S17]. |
| **TS SDK** | Possible, poor fit | See above: the secret ends up in UI JavaScript, or there's a Node sidecar. Fast major-version churn. The repo isn't public [S31][S32]. |
| **MCP client (`rmcp`)** | Possible, poor fit | `rmcp` 3.9.0 is the official Rust SDK and active [S30]. But OAuth needs a browser, tool results are LLM-shaped, uploads still need HTTP, calls block up to 180 s, fixtures would have to replay MCP sessions, and there's no stability promise [S10][S11][S15]. |

## Terms of service notes

- **§7.2:** keep API keys confidential. Don't "embed API keys in … client-side applications" [S17]. The app ships no key. Each user's own key lives in the OS credential store (KQ-05) and is used only in the Rust core.
- **§7.4:** integrating the API into your own applications is allowed, including commercially within the plan [S17].
- **§4.3:** "You own your Generated Assets". **§4.2:** you keep rights to uploads, and Scenario gets a hosting/processing licence. **§4.6:** non-Enterprise content may be used, de-identified, to improve the service [S17].
- **§3.2 / §3.6:** CU don't roll over and aren't refunded. **§5.2A:** no automated scripts while throttled. **§7.5:** 90 days' notice for breaking API changes [S17].

## Gaps and risks

- **Mask polarity isn't machine-readable.** It varies by model: transparent alpha, white, or black = edit [S22][S23]. Risk: a silently inverted edit. We need a per-model mask-convention table plus a fixture test for each model.
- **Reference roles aren't first-class.** Most models take one `referenceImages` array [S21][S24]. Mapping style/structure/identity onto model inputs is our own per-model table.
- **The capabilities schema drifts** (`conditionalValues` → `conditional`) [S32]. Parse leniently. An unknown input type should hide the control, not crash.
- **Region size limits are per model** (e.g. GPT Image ≤ 3840 px). The region extractor must read `inputs[].max`/`maxSize` [S1][S22].
- **No seed on some edit models** (GPT Image, Gemini) [S22][S24].
- **Uploads persist in the account** until deleted, or 60 days after the account ends [S17]. `hide: true` is available [S1].
- **Polling only.** Webhooks need a public HTTPS endpoint [S16].

## Unverified

- How long a signed `asset.url` and presigned upload URLs stay valid.
- Numeric rate limits and concurrency per plan (only field names are known).
- Whether a project-scoped API key can call `GET /teams/{teamId}/limits`.
- Any server-side job timeout.
- Whether `dryRun` is free for every model. Docs say "nothing is executed nor consumed", except for `ipDetection`.
- Whether the same seed reproduces an identical image.
- Whether ToS §7.5 also covers the MCP server and the SDK.
- Whether Scenario's CORS policy allows calls from a Tauri webview origin.
- Inpaint quality on battlemap regions per model. This needs one manual real call (KQ-24).
- The upload guide's example (an 8 MB file with `parts: 1`) contradicts its own 5 MB minimum part size [S7].
- Whether the bearer JWT scheme is meant for third-party clients.

## Sources

[S1]: https://cdn.cloud.scenario.com/static/api/openapi.json
[S2]: https://docs.scenario.com/api/resources/jobs/methods/retrieve
[S3]: https://docs.scenario.com/get-started/documentation/understanding-api-responses-and-errors
[S4]: https://help.scenario.com/articles/7934059476-api-usage-and-credits-compute-units
[S5]: https://docs.scenario.com/get-started/documentation/key-concepts-terminology/api-key-and-authentication
[S6]: https://docs.scenario.com/get-started/documentation/key-capabilities-at-a-glance
[S7]: https://docs.scenario.com/get-started/content/uploading-assets
[S8]: https://docs.scenario.com/api/resources/assets/methods/upload
[S9]: https://docs.scenario.com/get-started/content/retrieve-asset-url-by-asset-id
[S10]: https://mcp.scenario.com/docs
[S11]: https://mcp.scenario.com/docs/tools/model-run
[S12]: https://mcp.scenario.com/docs/tools/model-schema-get
[S13]: https://mcp.scenario.com/docs/tools/upload-asset
[S14]: https://mcp.scenario.com/docs/tools/asset-download
[S15]: https://mcp.scenario.com/docs/tools/jobs-wait
[S16]: https://docs.scenario.com/get-started/documentation/webhooks
[S17]: https://www.scenario.com/terms-and-conditions
[S18]: https://www.scenario.com/changelog?tag=mcp-server
[S19]: https://docs.scenario.com/get-started/documentation/integrating-with-applications-real-world-scenarios
[S20]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-scenario
[S21]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-bfl
[S22]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-openai
[S23]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-ideogram
[S24]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-google
[S25]: https://docs.scenario.com/get-started/generation/image-upscale-models
[S26]: https://docs.scenario.com/get-started/generation/background-removal-models
[S27]: https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-patina-ai/
[S28]: https://mcp.scenario.com/docs/tools/consumption-limit-get
[S29]: https://mcp.scenario.com/docs/tools/usage
[S30]: https://crates.io/crates/rmcp
[S31]: https://www.npmjs.com/package/@scenario-labs/sdk
[S32]: https://registry.npmjs.org/@scenario-labs/sdk
[S33]: https://docs.scenario.com/sdk-helpers

| Ref | Source |
|---|---|
| S1 | [Scenario REST API OpenAPI spec](https://cdn.cloud.scenario.com/static/api/openapi.json) |
| S2 | [Jobs: Retrieve](https://docs.scenario.com/api/resources/jobs/methods/retrieve) |
| S3 | [API responses and errors](https://docs.scenario.com/get-started/documentation/understanding-api-responses-and-errors) |
| S4 | [API usage and Compute Units](https://help.scenario.com/articles/7934059476-api-usage-and-credits-compute-units) |
| S5 | [API key and authentication](https://docs.scenario.com/get-started/documentation/key-concepts-terminology/api-key-and-authentication) |
| S6 | [Key capabilities at a glance](https://docs.scenario.com/get-started/documentation/key-capabilities-at-a-glance) |
| S7 | [Uploading assets](https://docs.scenario.com/get-started/content/uploading-assets) |
| S8 | [Assets: Upload](https://docs.scenario.com/api/resources/assets/methods/upload) |
| S9 | [Retrieve asset URL](https://docs.scenario.com/get-started/content/retrieve-asset-url-by-asset-id) |
| S10 | [Scenario MCP server docs](https://mcp.scenario.com/docs) |
| S11 | [MCP `model_run`](https://mcp.scenario.com/docs/tools/model-run) |
| S12 | [MCP `model_schema_get`](https://mcp.scenario.com/docs/tools/model-schema-get) |
| S13 | [MCP `upload_asset`](https://mcp.scenario.com/docs/tools/upload-asset) |
| S14 | [MCP `asset_download`](https://mcp.scenario.com/docs/tools/asset-download) |
| S15 | [MCP `jobs_wait`](https://mcp.scenario.com/docs/tools/jobs-wait) |
| S16 | [Webhooks](https://docs.scenario.com/get-started/documentation/webhooks) |
| S17 | [Terms and Conditions](https://www.scenario.com/terms-and-conditions) |
| S18 | [MCP server changelog](https://www.scenario.com/changelog?tag=mcp-server) |
| S19 | [Integrating with applications](https://docs.scenario.com/get-started/documentation/integrating-with-applications-real-world-scenarios) |
| S20 | [Scenario image models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-scenario) |
| S21 | [BFL models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-bfl) |
| S22 | [OpenAI models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-openai) |
| S23 | [Ideogram models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-ideogram) |
| S24 | [Google models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-google) |
| S25 | [Image upscale models](https://docs.scenario.com/get-started/generation/image-upscale-models) |
| S26 | [Background removal models](https://docs.scenario.com/get-started/generation/background-removal-models) |
| S27 | [PATINA models](https://docs.scenario.com/get-started/generation/third-party-model-generation/third-party-model-generation-patina-ai/) |
| S28 | [MCP `consumption_limit_get`](https://mcp.scenario.com/docs/tools/consumption-limit-get) |
| S29 | [MCP `usage`](https://mcp.scenario.com/docs/tools/usage) |
| S30 | [`rmcp` crate](https://crates.io/crates/rmcp) |
| S31 | [`@scenario-labs/sdk` on npm](https://www.npmjs.com/package/@scenario-labs/sdk) (README, published package source) |
| S32 | [`@scenario-labs/sdk` registry metadata and changelog](https://registry.npmjs.org/@scenario-labs/sdk) |
| S33 | [SDK helpers](https://docs.scenario.com/sdk-helpers) |
