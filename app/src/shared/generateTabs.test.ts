// R85 / R94: the shown Generate tab is remembered per project, across closing and reopening it
// and across a restart (the webview's storage).
import { afterEach, expect, test, vi } from 'vitest';

import { generateTab, NEW_IMAGE_TAB, REGION_TAB } from './generateTabs';

afterEach(() => {
  generateTab.forgetAll();
  generateTab.select(REGION_TAB);
});

test('R85 R94 the chosen Generate tab persists across closing and reopening a project', async () => {
  generateTab.forgetAll();
  generateTab.projectOpened('p1');
  expect(generateTab.get()).toBe(REGION_TAB);
  generateTab.select(NEW_IMAGE_TAB);

  // Another project starts on its own tab (none remembered: Region edit).
  generateTab.projectOpened('p2');
  expect(generateTab.get()).toBe(REGION_TAB);

  // Back to the first one: New image again.
  generateTab.projectOpened('p1');
  expect(generateTab.get()).toBe(NEW_IMAGE_TAB);

  // An explicit request (Generate new…, FR-GEN-01) still wins.
  generateTab.openNextWith(REGION_TAB);
  generateTab.projectOpened('p1');
  expect(generateTab.get()).toBe(REGION_TAB);
  generateTab.select(NEW_IMAGE_TAB);

  // After a restart (a fresh module, the same storage).
  vi.resetModules();
  const fresh = await import('./generateTabs');
  fresh.generateTab.projectOpened('p1');
  expect(fresh.generateTab.get()).toBe(NEW_IMAGE_TAB);
  fresh.generateTab.forgetAll();
});

test('R85 a broken storage entry only forgets', () => {
  localStorage.setItem('pf.generate-tab', '{not json');
  generateTab.projectOpened('p1');
  expect(generateTab.get()).toBe(REGION_TAB);
});
