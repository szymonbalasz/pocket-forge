// The prompt block (FR-PRM-04..12) on the mocked core: helper chips per kind, default ticks, the
// final prompt preview with part markers and skipped badges, Retry badges, tips, read-only.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'vitest';

import type { FakeTauri } from '../../api/mocks/tauri';
import { mockPrm, TestHost, userHelper, type HostLog } from './testUtils';

const BLEND =
  'Keep everything near the edges consistent with the surrounding map. Top-down battlemap view.';
const U1 = '01JPRMUSER000000000000000A';

let fake: FakeTauri;

afterEach(() => {
  cleanup();
  fake.uninstall();
});

const chips = () => screen.queryAllByTestId('prm-block-helper');
const chip = (id: string) =>
  chips().find((c) => c.getAttribute('data-record-id') === id) as HTMLButtonElement;
const ids = () => chips().map((c) => c.getAttribute('data-record-id'));
const pressed = () =>
  chips()
    .filter((c) => c.getAttribute('aria-pressed') === 'true')
    .map((c) => c.getAttribute('data-record-id'));
const finalText = () => screen.getByTestId('prm-block-final-prompt').textContent;

describe('Prompt block', () => {
  test('FR-PRM-04 chips list the visible helpers of the project kind, in settings order', async () => {
    fake = mockPrm();
    const { unmount } = render(<TestHost kind="map" />);
    await waitFor(() => expect(chips()).toHaveLength(4));
    expect(ids()).toEqual([
      'builtin:top-down-map',
      'builtin:match-style',
      'builtin:no-people',
      'builtin:no-grid-text',
    ]);
    expect(chip('builtin:no-people').textContent).toBe('No people or tokens');
    // The text is the tooltip (Radix, on focus).
    await act(async () => chip('builtin:no-people').focus());
    await waitFor(() =>
      expect(screen.getAllByText('No people, creatures or tokens.').length).toBeGreaterThan(0),
    );
    unmount();

    render(<TestHost kind="token" />);
    await waitFor(() => expect(chips()).toHaveLength(3));
    expect(ids()).toEqual(['builtin:token-single', 'builtin:token-plain-bg', 'builtin:no-text']);
  });

  test('FR-PRM-04 hidden helpers are not chips; the row follows prm:helpers-changed', async () => {
    fake = mockPrm();
    render(<TestHost kind="map" />);
    await waitFor(() => expect(chips()).toHaveLength(4));
    const list = await fake.callsOf('prm_helpers_list');
    expect(list).toHaveLength(1);
    // Settings hides one and adds a map helper: the event carries the new list.
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const rows = (await handlers.prm_helpers_list!({})) as { id: string; hidden: boolean }[];
    await act(async () => {
      await handlers.prm_helpers_save!({
        helpers: [
          ...rows.map((r) => (r.id === 'builtin:match-style' ? { ...r, hidden: true } : r)),
          { label: 'Moody', text: 'Moody torchlight.', kinds: ['map'], default_on: false },
        ],
      });
    });
    await waitFor(() => expect(chips()).toHaveLength(4));
    expect(ids()).not.toContain('builtin:match-style');
    expect(chips().at(-1)!.textContent).toBe('Moody');
  });

  test('FR-PRM-05 null ticks = default_on helpers; the first change writes the explicit list', async () => {
    fake = mockPrm({
      helpers: undefined,
    });
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const rows = (await handlers.prm_helpers_list!({})) as { id: string }[];
    await handlers.prm_helpers_save!({
      helpers: rows.map((r) => (r.id === 'builtin:no-people' ? { ...r, default_on: true } : r)),
    });
    const log: HostLog = { ticks: [], previews: [] };
    render(<TestHost kind="map" log={log} />);
    await waitFor(() => expect(pressed()).toEqual(['builtin:no-people']));
    expect(fake.callsOf('prm_compose_preview').at(-1)!.helper_ids).toBeNull();

    fireEvent.click(chip('builtin:no-grid-text'));
    expect(log.ticks).toEqual([['builtin:no-people', 'builtin:no-grid-text']]);
    await waitFor(() => expect(pressed()).toEqual(['builtin:no-people', 'builtin:no-grid-text']));
    fireEvent.click(chip('builtin:no-people'));
    expect(log.ticks.at(-1)).toEqual(['builtin:no-grid-text']);
    await waitFor(() =>
      expect(fake.callsOf('prm_compose_preview').at(-1)!.helper_ids).toEqual([
        'builtin:no-grid-text',
      ]),
    );
  });

  test('FR-PRM-05 an explicit empty list ticks nothing', async () => {
    fake = mockPrm({ helpers: [userHelper(U1, { default_on: true })] });
    render(<TestHost kind="map" initialTicks={[]} />);
    await waitFor(() => expect(chips().length).toBeGreaterThan(0));
    expect(pressed()).toEqual([]);
  });

  test('FR-PRM-08 the preview shows the exact text, marks each part and counts characters', async () => {
    fake = mockPrm();
    const log: HostLog = { ticks: [], previews: [] };
    render(
      <TestHost
        kind="map"
        prompt="a café"
        initialTicks={['builtin:no-grid-text', 'builtin:no-people']}
        blendInstruction={BLEND}
        log={log}
      />,
    );
    const want = `a café\n\nNo people, creatures or tokens. No grid lines, labels or text.\n\n${BLEND}`;
    await waitFor(() => expect(finalText()).toBe(want));
    expect(screen.getByTestId('prm-block-chars').textContent).toBe(
      `· ${[...want].length} characters`,
    );
    const parts = screen.getAllByTestId('prm-block-part');
    expect(parts.map((p) => p.getAttribute('data-source'))).toEqual([
      'typed',
      'helper',
      'helper',
      'blend',
    ]);
    expect(parts[0]!.getAttribute('title')).toBe('Typed prompt');
    expect(parts[1]!.getAttribute('title')).toBe('No people or tokens');
    expect(parts[1]!.getAttribute('data-record-id')).toBe('builtin:no-people');
    expect(parts[3]!.getAttribute('title')).toBe('Blend instruction');

    // Every keystroke updates it, without any provider call.
    fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: 'a forge' } });
    await waitFor(() => expect(finalText()).toBe(want.replace('a café', 'a forge')));
    expect(log.previews.at(-1)!.text).toBe(want.replace('a café', 'a forge'));
    expect(fake.calls.every((c) => c.cmd.startsWith('prm_') || c.cmd.startsWith('plugin:'))).toBe(
      true,
    );
  });

  test('FR-PRM-07 skipped helpers keep their chip ticked and show why', async () => {
    fake = mockPrm({
      helpers: undefined,
    });
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const rows = (await handlers.prm_helpers_list!({})) as object[];
    await handlers.prm_helpers_save!({
      helpers: [
        ...rows,
        { id: U1, label: 'Again', text: 'no people,  creatures or tokens', kinds: ['map'] },
      ],
    });
    render(
      <TestHost
        kind="map"
        initialTicks={['builtin:top-down-map', 'builtin:no-people', U1]}
        blendInstruction={BLEND}
      />,
    );
    await waitFor(() =>
      expect(chip('builtin:top-down-map').getAttribute('data-skipped')).toBe(
        'in_blend_instruction',
      ),
    );
    expect(
      within(chip('builtin:top-down-map')).getByTestId('prm-block-helper-skipped').textContent,
    ).toBe('In blend instruction');
    expect(chip(U1).getAttribute('data-skipped')).toBe('duplicate');
    expect(within(chip(U1)).getByTestId('prm-block-helper-skipped').textContent).toBe('Duplicate');
    expect(chip('builtin:no-people').getAttribute('data-skipped')).toBeNull();
    expect(pressed()).toContain('builtin:top-down-map');
    expect(finalText()!.match(/Top-down battlemap view\./g)).toHaveLength(1);
  });

  test('FR-PRM-08 an empty prompt with nothing ticked shows a dash and 0 characters', async () => {
    fake = mockPrm();
    render(<TestHost kind="item" initialTicks={[]} />);
    await waitFor(() =>
      expect(screen.getByTestId('prm-block-chars').textContent).toBe('· 0 characters'),
    );
    expect(screen.getByTestId('prm-block-final-prompt').getAttribute('data-empty')).toBe('true');
  });

  test('FR-PRM-10 Retry badges: changed since NNN on a ticked chip, missing helpers listed', async () => {
    fake = mockPrm();
    render(
      <TestHost
        kind="map"
        initialTicks={['builtin:no-people']}
        retry={{
          since: '001',
          changed: ['builtin:no-people'],
          missing: [{ id: U1, label: 'Moody' }],
        }}
      />,
    );
    await waitFor(() => expect(chips()).toHaveLength(4));
    expect(
      within(chip('builtin:no-people')).getByTestId('prm-block-helper-changed').textContent,
    ).toBe('changed since 001');
    expect(screen.getByTestId('prm-block-missing').textContent).toBe(
      'Missing since 001, not sent: Moody',
    );
    expect(pressed()).toEqual(['builtin:no-people']);
  });

  test('FR-PRM-11 placeholder per host, and three fixed tips that never change the prompt', async () => {
    fake = mockPrm();
    const { unmount } = render(<TestHost host="rgn" />);
    const field = screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement;
    expect(field.placeholder).toBe(
      "Describe the result, e.g. 'a kitchen with a stone floor and a hearth'",
    );
    await act(async () => fireEvent.click(screen.getByTestId('prm-block-tips')));
    const tips = await screen.findByTestId('prm-block-tips-content');
    const items = within(tips).getAllByRole('listitem');
    expect(items).toHaveLength(3);
    expect(items[0]!.textContent).toMatch(/^Describe the result, not an instruction/);
    expect(field.value).toBe('');
    unmount();

    render(<TestHost host="gen" />);
    expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).placeholder).toBe(
      'Describe the image',
    );
  });

  test('FR-PRM-12 read-only project: the helpers row is shown but disabled', async () => {
    fake = mockPrm();
    const log: HostLog = { ticks: [], previews: [] };
    render(<TestHost kind="map" readOnly log={log} />);
    await waitFor(() => expect(chips()).toHaveLength(4));
    expect(chips().every((c) => (c as HTMLButtonElement).disabled)).toBe(true);
    fireEvent.click(chip('builtin:no-people'));
    expect(log.ticks).toEqual([]);
    expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).readOnly).toBe(true);
  });

  test('PRM slice 6: the Chat button is enabled (it opens the chat window)', async () => {
    fake = mockPrm();
    render(<TestHost />);
    expect((screen.getByTestId('prm-block-chat') as HTMLButtonElement).disabled).toBe(false);
  });

  test('PRM host rows render between the helpers row and the preview', async () => {
    fake = mockPrm();
    render(
      <TestHost>
        <label data-testid="host-row">Blend with surroundings</label>
      </TestHost>,
    );
    await waitFor(() => expect(chips()).toHaveLength(4));
    const block = screen.getByTestId('prm-block');
    const order = [...block.querySelectorAll('[data-testid]')].map((e) =>
      e.getAttribute('data-testid'),
    );
    const at = (id: string) => order.indexOf(id);
    expect(at('prm-block-helpers')).toBeLessThan(at('host-row'));
    expect(at('host-row')).toBeLessThan(at('prm-block-final'));
  });
});
