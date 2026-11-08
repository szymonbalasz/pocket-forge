// Child-process helpers for the dev scripts.
import { spawnSync } from 'node:child_process';

/**
 * Kills a child and everything it started. On Windows a `shell: true` child is cmd.exe, and
 * `child.kill()` ends only that shell: the real process (Vite under corepack → pnpm) stays
 * alive, still attached to the console, and swallows the pane's typed input (BL-04). `taskkill
 * /T` ends the whole tree.
 */
export function killTree(child) {
  if (child.exitCode !== null || child.signalCode !== null || !child.pid) return;
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
  } else {
    child.kill();
  }
}
