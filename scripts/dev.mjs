#!/usr/bin/env node
// Runs the app for development: Vite on the worktree's port + `tauri dev`, with PF_DATA_DIR and
// PF_LIBRARY_DIR from .env.local (run `corepack pnpm env:init` first). Extra args go to the app
// process environment as-is, e.g. `PF_FAULT=after_temp corepack pnpm dev`.
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { portFree, requireEnvLocal, root } from './lib/env.mjs';
import { killTree } from './lib/proc.mjs';

const env = requireEnvLocal();
const port = Number(env.PF_VITE_PORT);
if (!(await portFree(port))) {
  console.error(`Port ${port} is in use. Is the app already running in this worktree?`);
  process.exit(1);
}

const childEnv = { ...process.env, ...env };
const app = join(root, 'app');
const shell = process.platform === 'win32';
// No child reads the console: Vite's key shortcuts would bind stdin (BL-04), and Ctrl+C reaches
// every process on the console anyway.
const stdio = ['ignore', 'inherit', 'inherit'];
const vite = spawn('corepack', ['pnpm', 'exec', 'vite', '--port', String(port), '--strictPort'], {
  cwd: app,
  env: childEnv,
  stdio,
  shell,
});
// The slot's dev URL, merged over tauri.conf.json through a config file (no shell quoting).
const config = join(root, '.pf-dev', 'tauri.dev.json');
writeFileSync(config, JSON.stringify({ build: { devUrl: `http://localhost:${port}`, beforeDevCommand: '' } }));
const tauri = spawn('corepack', ['pnpm', 'exec', 'tauri', 'dev', '--config', shell ? `"${config}"` : config], {
  cwd: app,
  env: childEnv,
  stdio,
  shell,
});

let stopping = false;
const stop = (code) => {
  if (stopping) return;
  stopping = true;
  killTree(vite);
  killTree(tauri);
  process.exit(code ?? 0);
};
tauri.on('exit', stop);
vite.on('exit', (code) => code && stop(code));
process.on('SIGINT', () => stop(130));
