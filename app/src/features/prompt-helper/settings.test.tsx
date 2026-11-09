// Settings → Prompt helpers (FR-PRM-01..03) on the mocked core: per-kind lists, edit and Save
// (the whole ordered list), Hide, Reset, Reset all, + Helper, Delete with an inline confirm,
// field validation and the `too_many` message.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'vitest';

import type { PromptHelper } from '../../api';
import type { FakeTauri } from '../../api/mocks/tauri';
import { settingsSections } from '../../shared/settings';
import { HelperSettings } from './HelperSettings';
import './index';
import { mockPrm, userHelper } from './testUtils';

const U1 = '01JPRMUSER000000000000000A';

let fake: FakeTauri;

afterEach(() => {
  cleanup();
  fake.uninstall();
});

const rows = () => screen.queryAllByTestId('prm-settings-helper');
const row = (id: string) => rows().find((r) => r.getAttribute('data-record-id') === id)!;
const rowIds = () => rows().map((r) => r.getAttribute('data-record-id'));
const field = (id: string, name: string) =>
  within(row(id)).getByTestId(`prm-settings-helper-${name}`) as HTMLInputElement;
const tab = (kind: string) =>
  screen.getAllByTestId('prm-settings-tab').find((t) => t.getAttribute('data-record-id') === kind)!;
const save = () => screen.getByTestId('prm-settings-save') as HTMLButtonElement;
const lastSave = () =>
  fake.callsOf('prm_helpers_save').at(-1)!.helpers as (Partial<PromptHelper> & { id?: string })[];

async function open() {
  render(<HelperSettings />);
  await waitFor(() => expect(rows().length).toBeGreaterThan(0));
}

describe('Prompt helper settings', () => {
  test('FR-PRM-02 the section is registered and lists helpers per asset type', async () => {
    fake = mockPrm();
    expect(settingsSections().find((s) => s.id === 'prm-helpers')?.title).toBe('Prompt helpers');
    await open();
    expect(rowIds()).toEqual([
      'builtin:top-down-map',
      'builtin:match-style',
      'builtin:no-people',
      'builtin:no-grid-text',
    ]);
    fireEvent.click(tab('item'));
    expect(rowIds()).toEqual(['builtin:item-single', 'builtin:item-plain-bg', 'builtin:no-text']);
    expect(save().disabled).toBe(true);
  });

  test('FR-PRM-03 edit label, text, kinds and default_on, then Save writes the whole list', async () => {
    fake = mockPrm();
    await open();
    fireEvent.change(field('builtin:no-people', 'label'), { target: { value: 'Empty rooms' } });
    fireEvent.change(field('builtin:no-people', 'text'), {
      target: { value: 'No people or monsters.' },
    });
    const kinds = within(row('builtin:no-people')).getAllByTestId('prm-settings-helper-kind');
    fireEvent.click(kinds.find((k) => k.getAttribute('data-kind') === 'token')!);
    fireEvent.click(field('builtin:no-people', 'default'));
    expect(screen.getByTestId('prm-settings-dirty')).toBeTruthy();
    await act(async () => fireEvent.click(save()));
    await waitFor(() =>
      expect(screen.getByTestId('prm-settings-result').textContent).toBe('Saved.'),
    );
    const sent = lastSave();
    expect(sent).toHaveLength(9);
    expect(sent[2]).toEqual({
      id: 'builtin:no-people',
      label: 'Empty rooms',
      text: 'No people or monsters.',
      kinds: ['map', 'token'],
      default_on: true,
      hidden: false,
    });
    expect(save().disabled).toBe(true);
  });

  test('FR-PRM-03 Hide a built-in (no Delete), Reset restores the starter and unhides it', async () => {
    fake = mockPrm();
    await open();
    expect(
      within(row('builtin:match-style')).queryByTestId('prm-settings-helper-delete'),
    ).toBeNull();
    expect(field('builtin:match-style', 'reset').disabled).toBe(true);
    fireEvent.change(field('builtin:match-style', 'text'), { target: { value: 'Worse.' } });
    fireEvent.click(field('builtin:match-style', 'hide'));
    expect(row('builtin:match-style').getAttribute('data-hidden')).toBe('true');
    await act(async () => fireEvent.click(save()));
    await waitFor(() => expect(lastSave()[1]!.hidden).toBe(true));

    await act(async () => fireEvent.click(field('builtin:match-style', 'reset')));
    await waitFor(() =>
      expect(field('builtin:match-style', 'text').value).toBe(
        'Match the existing art style, palette, line weight and lighting.',
      ),
    );
    expect(fake.callsOf('prm_helpers_reset').at(-1)).toEqual({ id: 'builtin:match-style' });
    expect(row('builtin:match-style').getAttribute('data-hidden')).toBeNull();
    expect(save().disabled).toBe(true);
  });

  test('FR-PRM-03 Reset keeps other unsaved edits', async () => {
    fake = mockPrm();
    await open();
    fireEvent.change(field('builtin:match-style', 'text'), { target: { value: 'Worse.' } });
    await act(async () => fireEvent.click(save()));
    fireEvent.change(field('builtin:no-people', 'text'), { target: { value: 'Unsaved.' } });
    await act(async () => fireEvent.click(field('builtin:match-style', 'reset')));
    await waitFor(() => expect(field('builtin:match-style', 'text').value).not.toBe('Worse.'));
    expect(field('builtin:no-people', 'text').value).toBe('Unsaved.');
    expect(save().disabled).toBe(false);
  });

  test('FR-PRM-03 Reset all asks inline, restores the built-ins and keeps user helpers', async () => {
    fake = mockPrm({
      helpers: undefined,
    });
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const list = (await handlers.prm_helpers_list!({})) as PromptHelper[];
    await handlers.prm_helpers_save!({
      helpers: [userHelper(U1), ...list.map((h, i) => (i === 0 ? { ...h, text: 'Edited.' } : h))],
    });
    await open();
    expect(rowIds()[0]).toBe(U1);
    fireEvent.click(screen.getByTestId('prm-settings-reset-all'));
    expect(fake.callsOf('prm_helpers_reset')).toHaveLength(0);
    await act(async () => fireEvent.click(screen.getByTestId('prm-settings-reset-all-confirm')));
    await waitFor(() => expect(rowIds().at(-1)).toBe(U1));
    expect(fake.callsOf('prm_helpers_reset')).toEqual([{ all: true }]);
    expect(field('builtin:top-down-map', 'text').value).toBe('Top-down battlemap view.');
  });

  test('FR-PRM-03 + Helper, then Delete with an inline confirm (user-made only)', async () => {
    fake = mockPrm();
    await open();
    fireEvent.click(tab('token'));
    fireEvent.click(screen.getByTestId('prm-settings-add'));
    const key = rowIds().at(-1)!;
    expect(key).toMatch(/^new-/);
    expect(save().disabled).toBe(true); // empty label and text
    expect(within(row(key)).getByTestId('prm-settings-helper-error').textContent).toMatch(/label/);
    fireEvent.change(field(key, 'label'), { target: { value: 'Snarling' } });
    fireEvent.change(field(key, 'text'), { target: { value: 'Snarling, teeth bared.' } });
    await act(async () => fireEvent.click(save()));
    await waitFor(() =>
      expect(screen.getByTestId('prm-settings-result').textContent).toBe('Saved.'),
    );
    const added = lastSave().at(-1)!;
    expect(added).toEqual({
      label: 'Snarling',
      text: 'Snarling, teeth bared.',
      kinds: ['token'],
      default_on: false,
      hidden: false,
    });
    const id = rowIds().at(-1)!;
    expect(id).not.toMatch(/^new-/);

    fireEvent.click(field(id, 'delete'));
    expect(within(row(id)).getByTestId('prm-settings-delete-ask')).toBeTruthy();
    fireEvent.click(within(row(id)).getByTestId('prm-settings-delete-confirm'));
    expect(rowIds()).not.toContain(id);
    await act(async () => fireEvent.click(save()));
    await waitFor(() => expect(lastSave().some((h) => h.id === id)).toBe(false));
  });

  test('FR-PRM-04 up/down reorders within the asset type and Save keeps the order', async () => {
    fake = mockPrm();
    await open();
    expect(field('builtin:top-down-map', 'up').disabled).toBe(true);
    fireEvent.click(field('builtin:no-people', 'up'));
    expect(rowIds()).toEqual([
      'builtin:top-down-map',
      'builtin:no-people',
      'builtin:match-style',
      'builtin:no-grid-text',
    ]);
    // Items: no-text moves above item-plain-bg, past no token helper in between.
    fireEvent.click(tab('item'));
    fireEvent.click(field('builtin:no-text', 'up'));
    expect(rowIds()).toEqual(['builtin:item-single', 'builtin:no-text', 'builtin:item-plain-bg']);
    await act(async () => fireEvent.click(save()));
    await waitFor(() => expect(fake.callsOf('prm_helpers_save')).toHaveLength(1));
    expect(lastSave().map((h) => h.id)).toEqual([
      'builtin:top-down-map',
      'builtin:no-people',
      'builtin:match-style',
      'builtin:no-grid-text',
      'builtin:token-single',
      'builtin:token-plain-bg',
      'builtin:item-single',
      'builtin:no-text',
      'builtin:item-plain-bg',
    ]);
  });

  test('FR-PRM-01 field limits: a label over 40 or no asset type blocks Save', async () => {
    fake = mockPrm();
    await open();
    fireEvent.change(field('builtin:no-people', 'label'), { target: { value: 'x'.repeat(41) } });
    expect(save().disabled).toBe(true);
    expect(
      within(row('builtin:no-people')).getByTestId('prm-settings-helper-error').textContent,
    ).toBe('A label is 1 to 40 characters');
    fireEvent.change(field('builtin:no-people', 'label'), { target: { value: 'Fine' } });
    expect(save().disabled).toBe(false);
    const kinds = within(row('builtin:no-people')).getAllByTestId('prm-settings-helper-kind');
    fireEvent.click(kinds.find((k) => k.getAttribute('data-kind') === 'map')!);
    // Stays listed on this tab while edited.
    expect(rowIds()).toContain('builtin:no-people');
    expect(save().disabled).toBe(true);
  });

  test('FR-PRM-01 at 60 helpers + Helper is off with the too_many message; a too_many save shows it', async () => {
    const many = Array.from({ length: 51 }, (_, i) =>
      userHelper(`01JPRMUSER0000000000000${String(i).padStart(3, '0')}`, { label: `H${i}` }),
    );
    fake = mockPrm();
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const list = (await handlers.prm_helpers_list!({})) as PromptHelper[];
    await handlers.prm_helpers_save!({ helpers: [...list, ...many] });
    await open();
    expect((screen.getByTestId('prm-settings-add') as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByTestId('prm-settings-full').textContent).toBe(
      'At most 60 prompt helpers. Delete one before adding another.',
    );

    // The core's answer is mapped the same way (e.g. another window added one meanwhile).
    handlers.prm_helpers_save = () =>
      Promise.reject({ code: 'too_many', message: 'at most 60 prompt helpers', field: 'helpers' });
    fireEvent.change(field('builtin:no-people', 'text'), { target: { value: 'Changed.' } });
    await act(async () => fireEvent.click(save()));
    await waitFor(() =>
      expect(screen.getByTestId('prm-settings-result').textContent).toBe(
        'At most 60 prompt helpers. Delete one before adding another.',
      ),
    );
  });

  test('FR-PRM-01 a field error from the core is shown on its row', async () => {
    fake = mockPrm();
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    await open();
    handlers.prm_helpers_save = () =>
      Promise.reject({
        code: 'invalid_input',
        message: 'a helper text is 1 to 500 characters',
        field: 'helpers[2].text',
      });
    fireEvent.change(field('builtin:no-people', 'text'), { target: { value: 'Changed.' } });
    await act(async () => fireEvent.click(save()));
    await waitFor(() =>
      expect(
        within(row('builtin:no-people')).getByTestId('prm-settings-helper-error').textContent,
      ).toBe('A helper text is 1 to 500 characters'),
    );
  });
  test('FR-PRM-02 L5 an unreadable section warns in Settings in plain words; Save clears it', async () => {
    fake = mockPrm();
    const message =
      "The prompt helper settings couldn't be read, so the starter helpers are used until you save them again.";
    let warnings = [{ code: 'prompt_helpers_invalid', message }];
    const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
    w.__pfMockHandlers.app_info = () => ({
      version: '0.0.0',
      provider_mode: 'fake',
      data_dir: '<data-dir>',
      library_overridden: true,
      warnings,
    });
    const save0 = w.__pfMockHandlers.prm_helpers_save as (i: never) => unknown;
    w.__pfMockHandlers.prm_helpers_save = ((i: never) => {
      warnings = [];
      return save0(i);
    }) as () => unknown;
    await open();
    await waitFor(() =>
      expect(screen.getByTestId('prm-settings-load-warning').textContent).toBe(message),
    );
    expect(screen.getByTestId('prm-settings-load-warning').textContent).not.toMatch(
      /invalid type|expected/,
    );
    fireEvent.change(field('builtin:no-people', 'label'), { target: { value: 'Empty rooms' } });
    fireEvent.click(save());
    await waitFor(() =>
      expect(screen.getByTestId('prm-settings-result').textContent).toBe('Saved.'),
    );
    expect(screen.queryByTestId('prm-settings-load-warning')).toBeNull();
    cleanup();
    await open();
    await act(async () => undefined);
    expect(screen.queryByTestId('prm-settings-load-warning')).toBeNull();
  });
});
