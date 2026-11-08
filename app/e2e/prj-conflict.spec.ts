// PRJ conflict view e2e on the mocked core (asset-project §7, §13 thin e2e; FR-PRJ-36, #39):
// the banner opens a two-pane view; a side still syncing can't be kept; Keep this resolves.
import { expect, test } from '@playwright/test';

import { calls, defaultSeed, mockCore, openFirstCard, type MockSeed } from './mock-core';

const layer = (id: string, kind: 'original' | 'patch') => ({
  id,
  kind,
  file: kind === 'patch' ? `.pforge/layers/${id}.png` : null,
  cx: 256,
  cy: 320,
  scale: 1,
  rotation: 0,
  opacity: 1,
  visible: true,
});

const seed: MockSeed = {
  ...defaultSeed,
  projects: [
    {
      ...defaultSeed.projects[0]!,
      warnings: [
        { code: 'conflict', message: 'This project was also changed on another machine.' },
      ],
      conflict: {
        source: 'copy',
        this: {
          side: 'this',
          file: 'project.json',
          readable: true,
          committed_at: '2026-11-07T10:00:00.000Z',
          seq: 4,
          machine: 'this',
          layers: [layer('01JLAYERA', 'original'), layer('01JLAYERB', 'patch')],
          newest_revision: {
            id: '01JREV1',
            seq: 1,
            created_at: '2026-11-07T10:00:00.000Z',
            reason: 'accept',
            label: 'moss',
          },
          missing_files: [],
          can_keep: true,
        },
        other: {
          side: 'other',
          file: 'project-OTHERPC.json',
          readable: true,
          committed_at: '2026-11-07T11:00:00.000Z',
          seq: 7,
          machine: 'other',
          layers: [layer('01JLAYERA', 'original'), layer('01JLAYERC', 'patch')],
          newest_revision: null,
          missing_files: ['.pforge/layers/01JLAYERC.png'],
          can_keep: false,
        },
      },
    },
  ],
};

test('FR-PRJ-36 the conflict view shows both versions; Still syncing disables Keep other; Keep this resolves', async ({
  page,
}) => {
  await mockCore(page, seed);
  await page.goto('/');
  await openFirstCard(page);
  const banner = page.locator('[data-testid="prj-panel-warning"][data-code="conflict"]');
  await expect(banner).toBeVisible();
  await page.getByTestId('prj-panel-conflict-open').click();

  const mine = page.getByTestId('prj-conflict-this');
  const theirs = page.getByTestId('prj-conflict-other');
  await expect(mine).toContainText('this machine');
  await expect(theirs).toContainText('another machine');
  await expect(page.getByTestId('prj-conflict-this-layers').locator('li')).toHaveCount(2);
  await expect(page.getByTestId('prj-conflict-other-syncing')).toContainText('Still syncing');
  await expect(page.getByTestId('prj-conflict-keep-other')).toBeDisabled();

  await page.getByTestId('prj-conflict-keep-this').click();
  await expect(page.getByTestId('prj-conflict')).toHaveCount(0);
  await expect(banner).toHaveCount(0);
  const resolve = (await calls(page)).find((c) => c.cmd === 'project_resolve_conflict');
  expect(resolve?.args).toEqual({ input: { keep: 'this' } });
});
