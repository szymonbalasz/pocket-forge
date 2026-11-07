/// <reference types="vitest/config" />
import { defineConfig, loadEnv } from 'vite';
import react from '@vitejs/plugin-react';

// The dev port comes from the worktree's slot (AGENTS §9): `corepack pnpm env:init` writes
// PF_VITE_PORT = 5200 + 10 × slot into <worktree>/.env.local.
export default defineConfig(({ mode }) => {
  const env = { ...loadEnv(mode, '..', 'PF_'), ...process.env };
  const port = Number(env.PF_VITE_PORT ?? 5200);
  return {
    plugins: [react()],
    envDir: '..',
    envPrefix: 'VITE_',
    clearScreen: false,
    server: { port, strictPort: true },
    preview: { port: port + 1, strictPort: true },
    build: { target: 'es2023', chunkSizeWarningLimit: 2000 },
    test: {
      environment: 'jsdom',
      include: ['src/**/*.test.{ts,tsx}'],
      restoreMocks: true,
    },
  };
});
