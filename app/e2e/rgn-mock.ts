// RGN e2e helpers on top of edt-mock / mock-core: installs the mock of every rgn_* command
// (`src/api/mocks/rgn.ts`, the same one Vitest uses) before the page loads, opens the mocked
// map, and drives the job through `window.__pfRgnMock`.
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, type Page } from '@playwright/test';

import type { Capabilities } from '../src/api';
import { installRgnMock, type RgnMockSeed } from '../src/api/mocks/rgn';
import { hook, MAP, openMap, stageBox, view } from './edt-mock';

export const ALL_TYPES = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../fixtures/rgn/capabilities-all-types.json', import.meta.url)),
    'utf8',
  ),
) as Capabilities;

export function rgnSeed(patch: Partial<RgnMockSeed> = {}): RgnMockSeed {
  return {
    image: MAP,
    models: [{ id: 'fake-all-types', name: 'All types' }],
    capabilities: { 'fake-all-types': ALL_TYPES },
    spend: { cap: 100, threshold: 10 },
    ...patch,
  };
}

/** Opens the mocked map with the rgn mock installed, at 2 image px per CSS px. */
export async function openRgn(page: Page, patch: Partial<RgnMockSeed> = {}): Promise<void> {
  // Init scripts run in order: this one first, so mock-core picks up its handlers.
  await page.addInitScript(installRgnMock, rgnSeed(patch));
  await openMap(page);
  await hook(page, 'setView', { scale: 0.5, tx: 0, ty: 0 });
  await expect(page.getByTestId('rgn-panel')).toBeVisible();
}

/** Drags a region with EDT's region tool (R), image px. */
export async function drawRegion(
  page: Page,
  from: [number, number],
  to: [number, number],
): Promise<void> {
  const b = await stageBox(page);
  const v = await view(page);
  const at = ([x, y]: [number, number]) => ({
    x: b.x + v.tx + x * v.scale,
    y: b.y + v.ty + y * v.scale,
  });
  await page.keyboard.press('r');
  const a = at(from);
  const c = at(to);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(c.x, c.y, { steps: 8 });
  await page.mouse.up();
}

type Driver = 'run' | 'finish' | 'fail' | 'setCancel';

/** Calls a `window.__pfRgnMock` driver (run, finish, fail, setCancel). */
export function drive(page: Page, fn: Driver, ...args: unknown[]): Promise<void> {
  return page.evaluate(
    ([f, a]) =>
      (window as unknown as { __pfRgnMock: Record<string, (...x: unknown[]) => void> }).__pfRgnMock[
        f as string
      ]!(...(a as unknown[])),
    [fn, args] as const,
  );
}

/** Commands the UI called with this name, newest last (their `input`). */
export async function inputsOf(page: Page, cmd: string): Promise<Record<string, unknown>[]> {
  return page.evaluate(
    (c) =>
      (
        window as unknown as { __pfMock: { calls: { cmd: string; args: { input?: unknown } }[] } }
      ).__pfMock.calls
        .filter((x) => x.cmd === c)
        .map((x) => (x.args?.input ?? {}) as Record<string, unknown>),
    cmd,
  );
}

/** Picks `label` in the Radix select inside the control `name`. */
export async function choose(page: Page, name: string, label: string): Promise<void> {
  await page
    .getByTestId('rgn-control-input')
    .and(page.locator(`[data-record-id="${name}"]`))
    .getByRole('combobox')
    .click();
  await page.getByRole('option', { name: label, exact: true }).click();
}
