// Shared helpers for the dev scripts: the worktree root, .env.local and the slot scheme.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { join } from 'node:path';

export const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim();
export const envFile = join(root, '.env.local');

/** Dev port of a slot (AGENTS §9). */
export const portOf = (slot) => 5200 + 10 * slot;

/** Parses .env.local (KEY=VALUE lines, # comments). Empty object if missing. */
export function readEnvLocal() {
  if (!existsSync(envFile)) return {};
  const out = {};
  for (const line of readFileSync(envFile, 'utf8').split(/\r?\n/)) {
    const m = /^\s*([A-Z0-9_]+)\s*=\s*(.*)\s*$/.exec(line);
    if (m) out[m[1]] = m[2];
  }
  return out;
}

/** Loads .env.local or exits with a hint. */
export function requireEnvLocal() {
  const env = readEnvLocal();
  if (!env.PF_SLOT) {
    console.error('No .env.local in this worktree. Run: corepack pnpm env:init');
    process.exit(1);
  }
  return env;
}

/** True if nothing listens on the port (IPv4 and IPv6 loopback). */
export function portFree(port) {
  const probe = (host) =>
    new Promise((resolve) => {
      const s = createServer();
      s.once('error', (e) => resolve(e.code === 'EAFNOSUPPORT' || e.code === 'EADDRNOTAVAIL'));
      s.once('listening', () => s.close(() => resolve(true)));
      s.listen(port, host);
    });
  return Promise.all([probe('127.0.0.1'), probe('::1')]).then((r) => r.every(Boolean));
}
