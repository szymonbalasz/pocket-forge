import { defineConfig, devices } from '@playwright/test';
import { loadEnv } from 'vite';

// UI e2e with mocked core commands (KQ-33): Vite dev server + Chromium, no Tauri.
// Port = the worktree's dev port + 1, so e2e can run while `pnpm dev` is up (AGENTS §9).
const env = { ...loadEnv('development', '..', 'PF_'), ...process.env };
const port = Number(env.PF_E2E_PORT ?? Number(env.PF_VITE_PORT ?? 5200) + 1);
const ci = !!process.env.CI;

export default defineConfig({
  testDir: 'e2e',
  fullyParallel: true,
  forbidOnly: ci,
  // One retry in CI; a test that only passes on retry is reported as flaky (AGENTS §5).
  retries: ci ? 1 : 0,
  reporter: ci
    ? [['list'], ['html', { open: 'never' }], ['json', { outputFile: 'test-results/results.json' }]]
    : [['list']],
  use: {
    baseURL: `http://localhost:${port}`,
    trace: 'retain-on-failure',
    // Locally on Windows use the installed Edge (WebView2's engine); CI installs Chromium.
    ...(ci || process.platform !== 'win32' ? {} : { channel: 'msedge' }),
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } },
    },
  ],
  webServer: {
    command: `corepack pnpm exec vite --port ${port} --strictPort`,
    url: `http://localhost:${port}`,
    reuseExistingServer: !ci,
    timeout: 60_000,
  },
});
