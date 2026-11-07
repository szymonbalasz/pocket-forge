#!/usr/bin/env node
// FR-EDT-26: the `window.__pfTest` hook must be absent from production bundles. Runs after every
// `vite build` (app `build` script, so also inside `tauri build` in CI). Fails if any emitted JS
// mentions the hook.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { root } from "./lib/env.mjs";

const dist = join(root, "app", "dist");
const hits = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (
      /\.(m?js|html)$/.test(name) &&
      readFileSync(p, "utf8").includes("__pfTest")
    )
      hits.push(p);
  }
};
walk(dist);
if (hits.length) {
  console.error(
    `✖ __pfTest found in the production build (FR-EDT-26):\n  ${hits.join("\n  ")}`,
  );
  process.exit(1);
}
console.log("✔ no __pfTest in the production build (FR-EDT-26)");
