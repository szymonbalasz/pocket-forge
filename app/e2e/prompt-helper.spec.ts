// PRM thin e2e on the mocked core (prompt-helper §13, KQ-33): the prompt block in the Region edit
// tab. Tick two helpers → the final prompt preview; the job sends them.
import { expect, test } from '@playwright/test';

import { drawRegion, openRgn } from './rgn-mock';

test('FR-PRM-04 FR-PRM-06 FR-PRM-08 ticking two helpers shows them in the final prompt; submit sends them', async ({
  page,
}) => {
  await openRgn(page);
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('edt-toolbar-mask-region').click();
  await page.getByTestId('prm-block-prompt').fill('A stone well');
  const chip = (id: string) =>
    page.locator(`[data-testid="prm-block-helper"][data-record-id="${id}"]`);
  await chip('builtin:no-people').click();
  await chip('builtin:no-grid-text').click();
  await expect(chip('builtin:no-people')).toHaveAttribute('aria-pressed', 'true');

  await page.getByTestId('prm-block-final').locator('summary').click();
  const shown = page.getByTestId('prm-block-final-prompt');
  await expect(shown).toContainText(
    'A stone well\n\nNo people, creatures or tokens. No grid lines, labels or text.\n\nKeep everything',
  );
  await expect(page.getByTestId('prm-block-chars')).toContainText('characters');

  // "Top-down battlemap" is already in the Maps blend instruction: badged, not sent twice.
  await chip('builtin:top-down-map').click();
  await expect(chip('builtin:top-down-map')).toHaveAttribute(
    'data-skipped',
    'in_blend_instruction',
  );
  await expect(shown).toContainText('Top-down battlemap view.');
  expect(((await shown.textContent()) ?? '').match(/Top-down battlemap view\./g)).toHaveLength(1);
});
