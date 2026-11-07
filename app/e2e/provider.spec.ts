// Provider settings and status chip (provider spec §7, §13 thin e2e), mocked core (KQ-33).
import { expect, test } from '@playwright/test';

import { calls } from './mock-core';
import { mockProvider, type ProviderSeed } from './provider-mock';

const base: ProviderSeed = {
  mode: 'scenario',
  stored: null,
  connectivity: 'unknown',
  testResult: 'auth',
  testDelayMs: 0,
};

test('FR-PRV-07 fake mode shows the fake badge with its balance', async ({ page }) => {
  await mockProvider(page, { ...base, mode: 'fake' });
  const chip = page.getByTestId('prv-header-chip');
  await expect(chip).toHaveAttribute('data-state', 'fake');
  await expect(chip).toHaveText('Fake provider · 1,234 fake');
});

test('FR-PRV-06 FR-PRV-07 save clears the fields, stores the key and shows its suffix', async ({
  page,
}) => {
  await mockProvider(page, base);
  const chip = page.getByTestId('prv-header-chip');
  await expect(chip).toHaveAttribute('data-state', 'not_configured');
  await chip.click();
  await page.getByTestId('prv-settings-key').fill('dummy-key-abcd');
  await page.getByTestId('prv-settings-secret').fill('dummy-secret');
  await page.getByTestId('prv-settings-team').fill('team_1');
  await page.getByTestId('prv-settings-save').click();
  await expect(page.getByTestId('prv-settings-result')).toHaveAttribute('data-state', 'ok');
  await expect(page.getByTestId('prv-settings-key')).toHaveValue('');
  await expect(page.getByTestId('prv-settings-secret')).toHaveValue('');
  await expect(chip).toHaveAttribute('data-state', 'configured');
  await expect(chip).toHaveText('Scenario · configured …abcd · unknown');
  const sent = (await calls(page)).filter((c) => c.cmd.startsWith('provider_'));
  expect(sent).toContainEqual({
    cmd: 'provider_credentials_set',
    args: { input: { key: 'dummy-key-abcd', secret: 'dummy-secret' } },
  });
  expect(sent).toContainEqual({
    cmd: 'provider_settings_set',
    args: { input: { team_id: 'team_1', project_id: null } },
  });

  await page.getByTestId('prv-settings-clear').click();
  await expect(chip).toHaveAttribute('data-state', 'not_configured');
  await expect(page.getByTestId('prv-settings-stored')).toHaveText('No key stored.');
});

test('FR-PRV-07 test connection shows a spinner, then auth error links to settings', async ({
  page,
}) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testDelayMs: 300 });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(page.getByTestId('prv-settings-spinner')).toBeVisible();
  await expect(page.getByTestId('prv-settings-result')).toHaveText(
    'Scenario rejected the key and secret.',
  );
  await expect(chip).toHaveAttribute('data-state', 'auth_error');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('prv-settings-panel')).toBeHidden();
  await chip.click();
  await expect(page.getByTestId('prv-settings-panel')).toBeVisible();
});

test('FR-PRV-07 an unreachable provider shows offline', async ({ page }) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testResult: 'offline' });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(chip).toHaveAttribute('data-state', 'offline');
  await expect(chip).toHaveText('offline · unknown');
});

test('FR-PRV-07 a working connection shows ok and the balance', async ({ page }) => {
  await mockProvider(page, { ...base, stored: 'k-wxyz', testResult: 'ok' });
  const chip = page.getByTestId('prv-header-chip');
  await chip.click();
  await page.getByTestId('prv-settings-test').click();
  await expect(page.getByTestId('prv-settings-result')).toHaveText('Connected.');
  await expect(chip).toHaveText('Scenario · ok · 1,234 CU');
});

test('FR-PRV-07 the rail Settings entry opens the provider settings; Esc closes them', async ({
  page,
}) => {
  await mockProvider(page, { ...base, mode: 'fake' });
  await page.getByTestId('app-rail-settings').click();
  await expect(page.getByTestId('prv-settings-panel')).toBeVisible();
  await expect(page.getByTestId('prv-settings-key')).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('prv-settings-panel')).toBeHidden();
});
