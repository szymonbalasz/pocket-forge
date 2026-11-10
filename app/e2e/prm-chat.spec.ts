// PRM thin e2e (prompt-helper §13) on the mocked core: open the chat → consent → send →
// proposal → Replace → the main prompt field updates. #61: the chat is a panel docked in the
// main window, so everything runs on one page; hiding the panel keeps the thread.
import { expect, test } from '@playwright/test';

import { calls } from './mock-core';
import { drawRegion, openRgn } from './rgn-mock';

test('FR-PRM-20 FR-PRM-21 FR-PRM-27 FR-PRM-29 open the chat panel → consent → send → proposal → Replace → the prompt field updates', async ({
  page,
}) => {
  await openRgn(page);
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('a room');

  await expect(page.getByTestId('prm-dock')).toHaveCount(0);
  await page.getByTestId('prm-block-chat').click();
  const chat = page.getByTestId('prm-dock');
  await expect(chat).toBeVisible();
  await expect(chat.getByTestId('prm-chat-fake')).toHaveText('Fake chat');
  await chat.getByTestId('prm-chat-consent-accept').click();
  await expect(chat.getByTestId('prm-chat-host')).toHaveText('Region edit');

  await chat.getByTestId('prm-chat-input').fill('make this a kitchen');
  await chat.getByTestId('prm-chat-send').click();
  const card = chat.getByTestId('prm-chat-proposal');
  await expect(card).toHaveCount(1);
  await expect(card.locator('code')).toHaveText('make this a kitchen');
  await expect(chat.getByTestId('prm-chat-usage')).toContainText('≈ $');

  await chat.getByTestId('prm-chat-proposal-replace').click();
  await expect(page.getByTestId('prm-block-prompt')).toHaveValue('make this a kitchen');
  await expect(card).toContainText('Inserted');

  // #61: the status-bar toggle hides and shows the panel; the thread is still there.
  await page.getByTestId('prm-dock-toggle').click();
  await expect(chat).toBeHidden();
  await page.getByTestId('prm-dock-toggle').click();
  await expect(card).toHaveCount(1);
  expect((await calls(page)).some((c) => c.cmd === 'prm_chat_session_end')).toBe(false);
});

test('FR-PRM-21 walk-61 L1 the dock leaves the canvas 480 px, follows the window and keeps the status bar on one line', async ({
  page,
}) => {
  await page.setViewportSize({ width: 1700, height: 900 });
  await openRgn(page);
  await page.getByTestId('prm-block-chat').click();
  const dock = page.getByTestId('prm-dock');
  await expect(dock).toBeVisible();
  const width = async (testId: string) => (await page.getByTestId(testId).boundingBox())!.width;
  // Drag the handle as far left as it goes: the dock stops where the canvas keeps 480 px.
  const h = (await page.getByTestId('prm-dock-resize').boundingBox())!;
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
  await page.mouse.down();
  await page.mouse.move(10, h.y + h.height / 2, { steps: 5 });
  await page.mouse.up();
  const wide = await width('prm-dock');
  expect(wide).toBeGreaterThan(400);
  expect(wide).toBeLessThanOrEqual(720);
  expect(await width('app-slot-canvas')).toBeGreaterThanOrEqual(479);
  const bar = page.getByTestId('app-status-bar');
  expect((await bar.boundingBox())!.height).toBeLessThan(40);
  // A smaller window: the dock narrows (never below 320), the bar stays one line, the toggle stays.
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect.poll(() => width('prm-dock')).toBe(320);
  expect((await bar.boundingBox())!.height).toBeLessThan(40);
  await expect(page.getByTestId('prm-dock-toggle')).toBeVisible();
  // Room again: back to the width the user chose.
  await page.setViewportSize({ width: 1700, height: 900 });
  await expect.poll(() => width('prm-dock')).toBe(wide);
});
