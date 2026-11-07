import { defineConfig } from '@playwright/test';

// Benches launch their own headed Edge per test (see tests/helpers.ts). Dev server: pnpm dev (port 5174).
export default defineConfig({
  testDir: 'tests',
  workers: 1,
  timeout: 300_000,
  reporter: [['list']],
});
