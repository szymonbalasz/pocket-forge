// ESLint (AGENTS §9): typescript-eslint, React hooks, and import boundaries:
// - only src/api/ may import @tauri-apps/*;
// - a feature imports another feature only through its index.ts, and never its internals;
// - layers point one way: shell → features → store → api → shared.
import { defineConfig } from 'eslint/config';
import boundaries from 'eslint-plugin-boundaries';
import reactHooks from 'eslint-plugin-react-hooks';
import tseslint from 'typescript-eslint';

export default defineConfig(
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      'src-tauri/**',
      'playwright-report/**',
      'test-results/**',
    ],
  },
  tseslint.configs.recommended,
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    ignores: ['src/api/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['@tauri-apps/*'],
              message: 'Only app/src/api/ talks to Tauri (AGENTS §9). Use the api wrapper.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { boundaries },
    settings: {
      'import/resolver': { node: { extensions: ['.ts', '.tsx', '.js'] } },
      'boundaries/include': ['src/**/*'],
      'boundaries/elements': [
        { type: 'api', pattern: 'src/api' },
        { type: 'store', pattern: 'src/store' },
        { type: 'shared', pattern: 'src/shared' },
        { type: 'shell', pattern: 'src/shell' },
        { type: 'feature', pattern: 'src/features/*', capture: ['feature'] },
      ],
    },
    rules: {
      'boundaries/dependencies': [
        'error',
        {
          default: 'disallow',
          message:
            '{{ from.element.type }} may not import {{ to.element.type }} here (AGENTS §9 boundaries). Features are imported only through their index.ts.',
          policies: [
            {
              from: { element: { type: 'shell' } },
              allow: { to: { element: { types: ['shell', 'api', 'store', 'shared'] } } },
            },
            {
              from: { element: { type: 'shell' } },
              allow: {
                to: { element: { type: 'feature' }, file: { path: '**/features/*/index.ts' } },
              },
            },
            {
              from: { element: { type: 'feature' } },
              allow: { to: { element: { types: ['api', 'store', 'shared'] } } },
            },
            {
              // ui-guidelines: the shared inline icon set lives in the shell.
              from: { element: { type: 'feature' } },
              allow: { to: { element: { type: 'shell' }, file: { path: '**/shell/icons.tsx' } } },
            },
            {
              // AGENTS §9: a feature uses another feature only through its index.ts.
              from: { element: { type: 'feature' } },
              allow: {
                to: { element: { type: 'feature' }, file: { path: '**/features/*/index.ts' } },
              },
            },
            {
              from: { element: { type: 'feature' } },
              allow: {
                to: {
                  element: {
                    type: 'feature',
                    captured: { feature: '{{ from.element.captured.feature }}' },
                  },
                },
              },
            },
            {
              from: { element: { type: 'store' } },
              allow: { to: { element: { types: ['api', 'shared'] } } },
            },
            {
              from: { element: { type: 'api' } },
              allow: { to: { element: { types: ['api', 'shared'] } } },
            },
            {
              from: { element: { type: 'shared' } },
              allow: { to: { element: { type: 'shared' } } },
            },
          ],
        },
      ],
    },
  },
);
