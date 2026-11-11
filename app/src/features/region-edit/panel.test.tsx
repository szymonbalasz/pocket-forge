import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import { api, type Capabilities } from '../../api';
import type { FakeTauri } from '../../api/mocks/tauri';
import type { RgnMockSeed } from '../../api/mocks/rgn';
import { projectStore } from '../../store/project';
import { editorFeature } from '../editor';
import { GeneratePanel } from './GeneratePanel';
import { IterationTracker } from './IterationTracker';
import { JobStatus } from './JobStatus';
import { FOLDER_MOVED_BLOCKER, rgn } from './store';
import { ALL_TYPES, mockControls, mockCore, polyfillRadix, SIMPLE } from './testUtils';

const Picker = editorFeature.slots.bottom!;
const REGION = { x: 400, y: 400, w: 512, h: 512 };
const wait = (ms: number) => act(() => new Promise((r) => setTimeout(r, ms)));
const ids = (testId: string) =>
  screen.queryAllByTestId(testId).map((e) => e.getAttribute('data-record-id'));
const control = (name: string) =>
  screen.getAllByTestId('rgn-control-input').find((e) => e.dataset.recordId === name);

let fake: FakeTauri;

async function setup(seed: Partial<RgnMockSeed> = {}, region = REGION) {
  polyfillRadix();
  fake = mockCore(seed);
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
      <div className="pf-bottom">
        <IterationTracker />
        <Picker />
      </div>
      <JobStatus />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  if (region) act(() => rgn.setRegion(region));
  await waitFor(() => expect(rgn.get().fit).not.toBeNull());
}

/** Waits for a ready estimate (debounced 500 ms). */
async function estimated() {
  await waitFor(() => expect(rgn.get().estimate.status).toBe('ready'), { timeout: 4000 });
}

async function typePrompt(text: string) {
  fireEvent.change(screen.getByTestId('prm-block-prompt'), { target: { value: text } });
}

/** Opens a Radix select and returns its option labels. */
async function openSelect(trigger: HTMLElement): Promise<string[]> {
  fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: 'mouse' });
  const list = await screen.findByRole('listbox');
  return within(list)
    .getAllByRole('option')
    .map((o) => o.textContent ?? '');
}

async function chooseOption(trigger: HTMLElement, label: string) {
  await openSelect(trigger);
  fireEvent.click(screen.getByRole('option', { name: label }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

beforeEach(() => {
  rgn.detach();
});
afterEach(() => {
  cleanup();
  rgn.detach();
  fake.uninstall();
});

describe('Generate panel (RGN slice 6)', { timeout: 20_000 }, () => {
  test('FR-RGN-07 the model picker lists provider_list_models; offline marks it stale', async () => {
    await setup();
    expect(fake.callsOf('provider_list_models')).toEqual([{ operation: 'edit_masked' }]);
    const trigger = screen.getByTestId('rgn-panel-model');
    expect(trigger.textContent).toContain('All types');
    expect(await openSelect(trigger)).toEqual(['All types', 'Simple']);
    expect(screen.queryByTestId('rgn-panel-stale')).toBeNull();
    cleanup();
    rgn.detach();
    fake.uninstall();
    await setup({ modelsStale: true });
    expect(screen.getByTestId('rgn-panel-stale')).toBeTruthy();
    // §7 offline: no estimate, submit disabled.
    expect(screen.getByTestId('rgn-panel-estimate').dataset.status).toBe('idle');
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(true);
  });

  test('FR-RGN-08 the panel renders exactly the fixture capabilities (every type) and its warning', async () => {
    await setup();
    // strength is hidden by the `mode = fast` rule; images, prompt and seed are not value controls.
    expect(ids('rgn-control-input')).toEqual([
      'num_samples',
      'guidance',
      'steps',
      'mode',
      'style',
      'hd',
      'negative_prompt',
    ]);
    const kinds = Object.fromEntries(
      screen.getAllByTestId('rgn-control-input').map((e) => [e.dataset.recordId, e.dataset.kind]),
    );
    expect(kinds).toEqual({
      num_samples: 'slider',
      guidance: 'slider',
      steps: 'number',
      mode: 'select',
      style: 'select',
      hd: 'toggle',
      negative_prompt: 'text',
    });
    // Slider + number field with the range; a number field with only the known bound.
    const guidance = within(control('guidance')!).getByRole('spinbutton') as HTMLInputElement;
    expect([guidance.min, guidance.max, guidance.step, guidance.value]).toEqual([
      '1',
      '20',
      '0.5',
      '7.5',
    ]);
    expect(within(control('guidance')!).getByRole('slider')).toBeTruthy();
    const steps = within(control('steps')!).getByRole('spinbutton') as HTMLInputElement;
    expect([steps.min, steps.max]).toEqual(['10', '']);
    expect(within(control('steps')!).queryByRole('slider')).toBeNull();
    expect(within(control('hd')!).getByRole('switch')).toBeTruthy();
    expect(screen.getByTestId('prm-block-prompt')).toBeTruthy();
    expect(screen.getByTestId('rgn-panel-seed')).toBeTruthy();
    expect(screen.getByTestId('rgn-panel-warning').textContent).toContain('"lora"');
    expect(await openSelect(within(control('mode')!).getByRole('combobox'))).toEqual([
      '—',
      'fast',
      'quality',
      'creative',
    ]);
    fireEvent.keyDown(screen.getByRole('listbox'), { key: 'Escape' });

    // Another model: only its controls, no warning (FR-RGN-07/08).
    await chooseOption(screen.getByTestId('rgn-panel-model'), 'Simple');
    await waitFor(() => expect(ids('rgn-control-input')).toEqual(['num_samples']));
    expect(screen.queryByTestId('rgn-panel-warning')).toBeNull();
  });

  test('FR-RGN-33 rules: hidden controls appear/vanish, allowed narrows, required blocks submit by name', async () => {
    await setup();
    await typePrompt('a kitchen');
    await chooseOption(within(control('mode')!).getByRole('combobox'), 'quality');
    await waitFor(() => expect(control('style')!.dataset.required).toBe('true'));
    expect(ids('rgn-control-input')).toContain('strength');
    const reason = screen.getByTestId('rgn-panel-submit-reason');
    expect(reason.textContent).toBe('Required: style.');
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(true);
    // No estimate while a required control is empty.
    expect(rgn.get().estimate.status).toBe('idle');

    await chooseOption(within(control('style')!).getByRole('combobox'), 'photo');
    await chooseOption(within(control('mode')!).getByRole('combobox'), 'creative');
    // photo isn't allowed with creative: reset with a notice, choices narrowed.
    await waitFor(() => expect(rgn.get().values.style).toBeNull());
    expect(screen.getByTestId('rgn-panel-warning').textContent).toMatch(/style: “photo”/);
    expect(await openSelect(within(control('style')!).getByRole('combobox'))).toEqual([
      'painterly',
      'ink',
    ]);
    fireEvent.click(screen.getByRole('option', { name: 'ink' }));
    await estimated();
    // Every change re-evaluates through the core.
    expect(fake.callsOf('rgn_controls_evaluate').length).toBeGreaterThanOrEqual(4);

    await chooseOption(within(control('mode')!).getByRole('combobox'), 'fast');
    await waitFor(() => expect(ids('rgn-control-input')).not.toContain('strength'));
  });

  test('FR-RGN-34 sent values: required or user-changed visible controls only; hidden never', async () => {
    await setup();
    await typePrompt('stone floor');
    act(() => rgn.setValue('strength', 0.4)); // hidden while mode = fast
    act(() => rgn.setValue('guidance', 9));
    await estimated();
    const draft = fake.callsOf('rgn_job_estimate').at(-1)!.draft as Record<string, unknown>;
    expect(draft.controls).toEqual({ guidance: 9 });
    expect(draft.region).toEqual({ inner: REGION, margin: 128 });
    expect(draft.mask_id).toBe('01JMASK000000000000000000');
    expect(draft.seed).toBeNull();
  });

  test('FR-RGN-12 estimate 500 ms after a cost-affecting change only; the prompt never re-estimates', async () => {
    await setup();
    await estimated();
    const n = () => fake.callsOf('rgn_job_estimate').length;
    expect(n()).toBe(1);
    await typePrompt('a new prompt');
    act(() => rgn.setValue('guidance', 12));
    act(() => rgn.setValue('negative_prompt', 'blurry'));
    await wait(800);
    expect(n()).toBe(1);
    // Two quick cost-affecting changes: one estimate (exact timing: the debounce unit test).
    act(() => rgn.setValue('num_samples', 3));
    act(() => rgn.setValue('num_samples', 4));
    await waitFor(() => expect(n()).toBe(2), { timeout: 3000 });
    await estimated();
    await wait(800);
    expect(n()).toBe(2);
    expect(screen.getByTestId('rgn-panel-estimate').textContent).toContain('4 CU');
    // The margin changes the sent size: re-estimates too.
    act(() => rgn.setMargin(64));
    await waitFor(() => expect(n()).toBe(3), { timeout: 3000 });
  });

  test('FR-RGN-12 a failed estimate offers Retry, which estimates again with no input change', async () => {
    await setup();
    mockControls().failNext('rgn_job_estimate', 'rate_limited', 'Too many requests.');
    await waitFor(() => expect(rgn.get().estimate.status).toBe('error'), { timeout: 4000 });
    const n = fake.callsOf('rgn_job_estimate').length;
    fireEvent.click(screen.getByTestId('rgn-panel-estimate-retry'));
    await estimated();
    expect(fake.callsOf('rgn_job_estimate').length).toBe(n + 1);
    expect(screen.queryByTestId('rgn-panel-estimate-retry')).toBeNull();
  });

  test('FR-RGN-02 a cleared margin field puts the margin back on blur, never 0', async () => {
    await setup();
    const fits = fake.callsOf('rgn_region_fit').length;
    const field = screen.getByTestId('rgn-panel-margin') as HTMLInputElement;
    fireEvent.change(field, { target: { value: '' } });
    fireEvent.blur(field);
    expect(rgn.get().margin).toBe(128);
    expect(field.value).toBe('128');
    await wait(50);
    expect(fake.callsOf('rgn_region_fit').length).toBe(fits);
  });

  test('FR-RGN-05 fit label recomputed on region and margin change', async () => {
    await setup();
    // 512 + 2×128 = 768, a multiple of 8 and ≤ 2048: unresampled.
    expect(screen.getByTestId('rgn-panel-fit').textContent).toBe('sent unresampled');
    // Outer 2048 × 2528 (clamped at the image's right edge) > 2048: downscaled.
    act(() => rgn.setRegion({ x: 0, y: 0, w: 2000, h: 2400 }));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-panel-fit').textContent).toBe('patch 1.2× softer'),
    );
    expect(fake.callsOf('rgn_region_fit').at(-1)).toMatchObject({
      margin: 128,
      model_id: 'fake-all-types',
    });
  });

  test('FR-RGN-09 owner-55 blend off: the final prompt is sent byte for byte; submit disabled while empty', async () => {
    await setup({ spend: { cap: 100, threshold: 10 } });
    await estimated();
    expect(screen.getByTestId('rgn-panel-submit-reason').textContent).toBe('Required: prompt.');
    const prompt = '  Turn this room into a kitchen,\nwith copper pots  ';
    await typePrompt(prompt);
    fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
    expect(rgn.get().blendInstruction).toBe(false);
    await waitFor(() =>
      expect(screen.getByTestId('prm-block-final-prompt').textContent).toBe(prompt),
    );
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    const sent = fake.callsOf('rgn_job_submit')[0]!.draft as Record<string, unknown>;
    expect(sent.final_prompt).toBe(prompt);
    expect(sent.blend_instruction).toBe(false);
  });

  test('BL-39 an empty prompt shows as empty with "+ blend instruction at send", not the instruction alone', async () => {
    await setup();
    await waitFor(() => expect(rgn.get().blendText).not.toBeNull());
    const shown = () => screen.getByTestId('prm-block-final-prompt');
    await waitFor(() =>
      expect(screen.getByTestId('prm-block-final-note').textContent).toBe(
        '+ blend instruction at send',
      ),
    );
    expect(shown().textContent).toBe('—');
    expect(shown().dataset.empty).toBe('true');
    expect(screen.getByTestId('prm-block-final').textContent).not.toMatch(/Keep everything/);
    // Blend off: just the empty prompt.
    fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
    await waitFor(() => expect(screen.queryByTestId('prm-block-final-note')).toBeNull());
    // A typed prompt shows as sent again.
    fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
    await typePrompt('a stone well');
    await waitFor(() =>
      expect(shown().textContent).toMatch(/^a stone well\n\nKeep everything near the edges/),
    );
    expect(screen.queryByTestId('prm-block-final-note')).toBeNull();
  });

  test('owner-55 blend instruction: on by default, shown once in the final prompt, saved in the draft, Settings text used', async () => {
    await setup();
    const toggle = screen.getByTestId('rgn-panel-blend-instruction');
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    await typePrompt('a stone well');
    await waitFor(() => expect(rgn.get().blendText).not.toBeNull());
    expect(screen.getByTestId('prm-block-final').textContent).toMatch(/^Final prompt \(as sent\)/);
    const shown = () => screen.getByTestId('prm-block-final-prompt').textContent ?? '';
    await waitFor(() => expect(shown()).toMatch(/^a stone well\n\nKeep everything near the edges/));
    expect(shown()).toMatch(/toward the centre of the edited area\. Top-down battlemap view\.$/);
    expect(shown().match(/battlemap/g)).toHaveLength(1);
    // Not cost-affecting: toggling never re-estimates.
    await estimated();
    const n = fake.callsOf('rgn_job_estimate').length;
    fireEvent.click(toggle);
    await waitFor(() => expect(shown()).toBe('a stone well'));
    await waitFor(
      () =>
        expect(fake.callsOf('project_put_draft').at(-1)?.draft).toMatchObject({
          blend_instruction: false,
        }),
      { timeout: 4000 },
    );
    fireEvent.click(toggle);
    await wait(600);
    expect(fake.callsOf('rgn_job_estimate').length).toBe(n);
    // A new Settings text reaches the preview (`rgn:blend-instruction`).
    await act(() => api.region.setBlendInstruction('Match the ink.'));
    await waitFor(() =>
      expect(shown()).toBe('a stone well\n\nMatch the ink. Top-down battlemap view.'),
    );
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    // The core appends at submit; the panel sends the typed prompt and the setting.
    expect(fake.callsOf('rgn_job_submit')[0]!.draft).toMatchObject({
      final_prompt: 'a stone well',
      blend_instruction: true,
    });
  });

  test('FR-RGN-08 owner-2026-11-08 a model without a count control shows 1 result per job', async () => {
    const one: Capabilities = {
      ...SIMPLE,
      model_id: 'fake-one',
      controls: SIMPLE.controls.filter((c) => c.role !== 'candidate_count'),
    };
    await setup({
      models: [
        { id: 'fake-simple', name: 'Simple' },
        { id: 'fake-one', name: 'One' },
      ],
      capabilities: { 'fake-simple': SIMPLE, 'fake-one': one },
    });
    await waitFor(() => expect(ids('rgn-control-input')).toEqual(['num_samples']));
    expect(screen.queryByTestId('rgn-panel-count-single')).toBeNull();
    await chooseOption(screen.getByTestId('rgn-panel-model'), 'One');
    await waitFor(() => expect(screen.queryByTestId('rgn-panel-count-single')).not.toBeNull());
    expect(ids('rgn-control-input')).toEqual([]);
    expect(screen.getByTestId('rgn-panel-count-single').textContent).toBe(
      '1 result per job (this model)',
    );
    await typePrompt('a well');
    await estimated();
    expect(rgn.get().estimate.result?.candidate_count).toBe(1);
    expect(fake.callsOf('rgn_job_estimate').at(-1)!.draft).toMatchObject({ controls: {} });
  });

  test('FR-RGN-03 owner-2026-11-08 no painted mask: Generate disabled with the mask hint', async () => {
    await setup({ maskId: null });
    await typePrompt('a well');
    const hint = () => screen.queryByTestId('rgn-panel-mask-hint');
    const submit = () => screen.getByTestId('rgn-panel-submit') as HTMLButtonElement;
    expect(hint()?.textContent).toBe('Paint a mask inside the region to choose what changes.');
    expect(submit().disabled).toBe(true);
    expect(screen.queryByTestId('rgn-panel-submit-reason')).toBeNull();
    // EDT reports paint before the hand-off: no hint, waiting for the mask save.
    act(() => rgn.setMaskPainted(true));
    expect(hint()).toBeNull();
    expect(screen.getByTestId('rgn-panel-submit-reason').textContent).toBe('Saving the mask…');
  });

  test('FR-RGN-03 owner-2026-11-08 a stored mask that EDT reports empty (cleared) shows the hint, no estimate', async () => {
    await setup();
    await typePrompt('a well');
    await estimated();
    const n = fake.callsOf('rgn_job_estimate').length;
    act(() => rgn.setMaskPainted(false));
    expect(screen.getByTestId('rgn-panel-mask-hint')).toBeTruthy();
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(true);
    await wait(600);
    expect(fake.callsOf('rgn_job_estimate').length).toBe(n);
    act(() => rgn.setMaskPainted(true));
    expect(screen.queryByTestId('rgn-panel-mask-hint')).toBeNull();
  });

  test('FR-RGN-10 the seed is random (null) or pinned, and the pinned value is sent', async () => {
    await setup();
    await typePrompt('x');
    await estimated();
    expect((fake.callsOf('rgn_job_estimate').at(-1)!.draft as { seed: unknown }).seed).toBeNull();
    fireEvent.click(screen.getByTestId('rgn-panel-seed-pin'));
    fireEvent.change(screen.getByTestId('rgn-panel-seed-value'), { target: { value: '42' } });
    expect(rgn.get().seed).toBe(42);
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect((fake.callsOf('rgn_job_submit')[0]!.draft as { seed: unknown }).seed).toBe(42);
  });

  test('FR-RGN-38 read-only: no estimates, submit disabled with the reason', async () => {
    await setup({ readOnly: true });
    await typePrompt('x');
    await wait(700);
    expect(fake.callsOf('rgn_job_estimate')).toHaveLength(0);
    expect(screen.getByTestId('rgn-panel-readonly')).toBeTruthy();
    expect(screen.getByTestId('rgn-panel-submit-reason').textContent).toBe(
      'The project is read-only.',
    );
  });
});

describe('confirm dialog (FR-RGN-14..16, §7)', { timeout: 20_000 }, () => {
  async function ready(seed: Partial<RgnMockSeed>, samples?: number) {
    await setup(seed);
    await typePrompt('kitchen');
    if (samples) act(() => rgn.setValue('num_samples', samples));
    await estimated();
    if (samples) await waitFor(() => expect(rgn.get().estimate.result?.cost.amount).toBe(samples));
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    return screen.findByTestId('rgn-confirm');
  }

  test('FR-RGN-14 within limits: shows estimate, spend after and cap; Enter confirms', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    expect(within(dlg).getByTestId('rgn-confirm-estimate').textContent).toBe('2 CU');
    expect(within(dlg).getByTestId('rgn-confirm-spend').textContent).toBe('2 CU');
    expect(dlg.textContent).toContain('100 CU');
    expect(document.activeElement).toBe(within(dlg).getByTestId('rgn-confirm-ok'));
    fireEvent.submit(dlg.querySelector('form')!);
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(fake.callsOf('rgn_job_submit')[0]!.confirmations).toEqual([]);
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
  });

  test('FR-RGN-14 the confirm closes when its estimate goes away and doesn’t come back by itself', async () => {
    await ready({ spend: { cap: 100, threshold: 10 } });
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const status = (connectivity: string) => () => ({
      mode: 'fake',
      configured: true,
      connectivity,
      store_unavailable: false,
    });
    handlers.provider_status = status('offline');
    act(() => fake.emit('provider:status-changed', { connectivity: 'offline' }));
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    handlers.provider_status = status('ok');
    act(() => fake.emit('provider:status-changed', { connectivity: 'ok' }));
    await estimated();
    await wait(50);
    expect(screen.queryByTestId('rgn-confirm')).toBeNull();
    expect(rgn.get().confirmOpen).toBe(false);
  });

  test('FR-RGN-15 above the threshold: Enter never confirms; the second confirmation is explicit', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 3 } }, 4);
    expect(dlg.dataset.overThreshold).toBe('true');
    expect(document.activeElement).toBe(within(dlg).getByTestId('rgn-confirm-cancel'));
    const ok = within(dlg).getByTestId('rgn-confirm-ok') as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-second'));
    expect(ok.disabled).toBe(false);
    fireEvent.submit(dlg.querySelector('form')!); // Enter in the form
    const enterOnOk = fireEvent.keyDown(ok, { key: 'Enter' });
    expect(enterOnOk).toBe(false); // default (the click) prevented
    await wait(50);
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(0);
    fireEvent.click(ok);
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(fake.callsOf('rgn_job_submit')[0]!.confirmations).toEqual(['threshold']);
  });

  test('FR-RGN-15 an unknown cost always asks twice', async () => {
    const dlg = await ready({ costPerSample: 'unknown' });
    expect(dlg.dataset.unknownCost).toBe('true');
    expect(within(dlg).getByTestId('rgn-confirm-estimate').textContent).toBe('unknown');
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-second'));
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(fake.callsOf('rgn_job_submit')[0]!.confirmations).toEqual(['unknown_cost']);
  });

  test('FR-RGN-14 over the cap: spend, estimate and cap shown; an explicit raise is stored first', async () => {
    const dlg = await ready({ spend: { cap: 3, threshold: 10 } }, 4);
    expect(dlg.dataset.overCap).toBe('true');
    expect(within(dlg).getByTestId('rgn-confirm-over-cap').textContent).toMatch(
      /over the cap of 3 CU/,
    );
    const ok = within(dlg).getByTestId('rgn-confirm-ok') as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    expect(document.activeElement).toBe(within(dlg).getByTestId('rgn-confirm-cap'));
    fireEvent.change(within(dlg).getByTestId('rgn-confirm-cap'), { target: { value: '20' } });
    expect(ok.disabled).toBe(false);
    fireEvent.click(ok);
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(fake.callsOf('rgn_spend_set')).toEqual([{ cap: 20, threshold: 10 }]);
    expect(mockControls().state().spend).toBe(4);
  });

  test('FR-RGN-16 first submit with no cap or threshold asks for both and stores them', async () => {
    const dlg = await ready({ spend: { cap: null, threshold: null } });
    expect(dlg.dataset.setup).toBe('true');
    const ok = within(dlg).getByTestId('rgn-confirm-ok') as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    fireEvent.change(within(dlg).getByTestId('rgn-confirm-cap'), { target: { value: '50' } });
    fireEvent.change(within(dlg).getByTestId('rgn-confirm-threshold'), { target: { value: '5' } });
    fireEvent.click(ok);
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(fake.callsOf('rgn_spend_set')).toEqual([{ cap: 50, threshold: 5 }]);
    expect(fake.callsOf('rgn_job_submit')[0]!.confirmations).toEqual(['set_cap']);
  });

  test('FR-RGN-16 walk-61 L3: once set, the next submit of the same draft doesn’t ask for the limits again', async () => {
    const dlg = await ready({ spend: { cap: null, threshold: null } });
    fireEvent.change(within(dlg).getByTestId('rgn-confirm-cap'), { target: { value: '50' } });
    fireEvent.change(within(dlg).getByTestId('rgn-confirm-threshold'), { target: { value: '5' } });
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    act(() => mockControls().finish(2));
    const submit = () => screen.getByTestId('rgn-panel-submit') as HTMLButtonElement;
    await waitFor(() => expect(submit().disabled).toBe(false));
    // The same draft: the estimate from before the limits were set is reused.
    fireEvent.click(submit());
    const again = await screen.findByTestId('rgn-confirm');
    expect(again.dataset.setup).toBe('false');
    expect(within(again).queryByTestId('rgn-confirm-cap')).toBeNull();
    expect(again.textContent).toContain('50 CU');
    fireEvent.click(within(again).getByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(2));
    expect(fake.callsOf('rgn_job_submit')[1]!.confirmations).toEqual([]);
    expect(fake.callsOf('rgn_spend_set')).toHaveLength(1);
  });

  test('§7 cancel closes without submitting; Esc too', async () => {
    let dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-cancel'));
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    dlg = await screen.findByTestId('rgn-confirm');
    fireEvent.keyDown(dlg, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(0);
  });

  const handlers = () =>
    (window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> })
      .__pfMockHandlers;

  test('FR-RGN-13 BL-36 an estimate expired while the dialog was open: re-estimates and submits within what was confirmed', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    mockControls().failNext('rgn_job_submit', 'estimate_stale', 'The estimate is out of date.');
    const before = fake.callsOf('rgn_job_estimate').length;
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(2));
    expect(fake.callsOf('rgn_job_estimate').length).toBe(before + 1);
    // The id hashes the inputs, so the fresh estimate (re-stored, not expired) has the same one.
    const second = fake.callsOf('rgn_job_submit')[1] as { estimate_id: string };
    expect(second.estimate_id).toBe(rgn.get().estimate.result!.estimate_id);
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    expect(screen.queryByTestId('rgn-panel-submit-error')).toBeNull();
    expect(mockControls().state().spend).toBe(2);
  });

  test('walk owner-UX L2: a moved folder blocks Generate with a readable reason, no OS text', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    const h = handlers();
    h.rgn_job_submit = () =>
      Promise.reject({
        code: 'not_found',
        message:
          'The project folder was moved or renamed outside Pocket Forge. Reopen it from home.',
        detail: { reason: 'folder_moved' },
      });
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(screen.queryByTestId('rgn-confirm')).toBeNull());
    const submit = screen.getByTestId('rgn-panel-submit') as HTMLButtonElement;
    await waitFor(() => expect(submit.disabled).toBe(true));
    expect(screen.getByTestId('rgn-panel-submit-reason').textContent).toBe(FOLDER_MOVED_BLOCKER);
    expect(screen.getByTestId('rgn-panel-submit-error').textContent).not.toMatch(/os error/);
    // No estimates while it's moved.
    expect(rgn.get().estimate.status).toBe('idle');
  });

  test('BL-36 a dearer fresh estimate shows the confirm again; nothing is spent', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    mockControls().failNext('rgn_job_submit', 'estimate_stale', 'The estimate is out of date.');
    const h = handlers();
    const estimate = h.rgn_job_estimate!;
    h.rgn_job_estimate = async (i) => {
      const r = (await estimate(i)) as { cost: { amount: number } };
      return { ...r, estimate_id: 'E-dearer', cost: { ...r.cost, amount: r.cost.amount + 1 } };
    };
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    const again = await screen.findByText(/estimate changed while this dialog was open/);
    expect(again).not.toBeNull();
    expect(
      within(screen.getByTestId('rgn-confirm')).getByTestId('rgn-confirm-estimate').textContent,
    ).toBe('3 CU');
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
    expect(mockControls().state().spend).toBe(0);
    h.rgn_job_estimate = estimate;
  });

  test('FR-RGN-13 stale twice: the dialog closes and the panel estimates again', async () => {
    const dlg = await ready({ spend: { cap: 100, threshold: 10 } });
    const h = handlers();
    const submit = h.rgn_job_submit!;
    h.rgn_job_submit = () => {
      throw { code: 'estimate_stale', message: 'The estimate is out of date.' };
    };
    fireEvent.click(within(dlg).getByTestId('rgn-confirm-ok'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-panel-submit-error').textContent).toMatch(/estimating again/),
    );
    expect(screen.queryByTestId('rgn-confirm')).toBeNull();
    h.rgn_job_submit = submit;
  });
});

describe('job strip, iteration tracker and picker (FR-RGN-18..21, 40)', { timeout: 20_000 }, () => {
  async function submitted(
    seed: Partial<RgnMockSeed> = {},
    prompt = 'Turn this room into a kitchen',
  ) {
    await setup({ spend: { cap: 100, threshold: 10 }, ...seed });
    await typePrompt(prompt);
    await estimated();
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')).toHaveLength(1));
  }

  test('FR-RGN-18 the job strip shows state, progress, elapsed and cost; status bar too', async () => {
    await submitted();
    expect(screen.getByTestId('rgn-job-state').textContent).toBe('Submitted');
    expect(screen.getByTestId('rgn-job-cost').textContent).toBe('2 CU');
    expect(screen.getByTestId('rgn-job-elapsed').textContent).toMatch(/^0:0\d$/);
    expect(screen.getByTestId('rgn-status-job').textContent).toBe('Job: Submitted');
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(true);
    act(() => mockControls().run(0.5));
    await waitFor(() => expect(screen.getByTestId('rgn-job-progress').textContent).toBe('50 %'));
    expect(screen.getByTestId('rgn-status-job').textContent).toBe('Job: Running 50 %');
    expect(screen.queryByTestId('rgn-job-long')).toBeNull();
    act(() => mockControls().run(0.6, { long_running: true }));
    await waitFor(() => expect(screen.getByTestId('rgn-job-long')).toBeTruthy());
  });

  test('FR-RGN-19 cancel: acked → canceled; not cancellable → the panel says so', async () => {
    await submitted();
    act(() => mockControls().setCancel('not_cancellable'));
    fireEvent.click(screen.getByTestId('rgn-job-cancel'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(/can’t be canceled/),
    );
    expect(screen.getByTestId('rgn-job-state').textContent).toBe('Submitted');
    act(() => mockControls().setCancel('canceled'));
    fireEvent.click(screen.getByTestId('rgn-job-cancel'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-state').textContent).toBe('Canceled'));
    expect(screen.queryByTestId('rgn-job-cancel')).toBeNull();
    expect(screen.getByTestId('rgn-job-retry')).toBeTruthy();
  });

  test('FR-RGN-20 a failed job shows the error and hint; insufficient_balance its own message; no resubmit', async () => {
    await submitted();
    act(() => mockControls().fail('insufficient_balance', 'Top up.'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-job-error').dataset.code).toBe('insufficient_balance'),
    );
    expect(screen.getByTestId('rgn-job-error').textContent).toMatch(/Not enough provider balance/);
    await wait(700);
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
  });

  test('FR-RGN-24 R102 a fetch error keeps the job succeeded: error shown, Fetch again, no resubmit', async () => {
    await submitted();
    act(() => mockControls().fetchError('provider_failure', 'Download failed.'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-job-fetch-error').dataset.code).toBe('provider_failure'),
    );
    expect(screen.getByTestId('rgn-job-state').textContent).toBe('Fetching');
    expect(screen.queryByTestId('rgn-job-retry')).toBeNull();
    fireEvent.click(screen.getByTestId('rgn-job-fetch-again'));
    await waitFor(() => expect(fake.callsOf('rgn_job_fetch_again')).toHaveLength(1));
    // As pf-core: the error stays until the candidates are stored.
    expect(screen.getByTestId('rgn-job-fetch-error')).toBeTruthy();
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getByTestId('rgn-job-state').textContent).toBe('Ready'));
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
  });

  test('FR-RGN-24 FR-RGN-37 Fetch again is refused while the spool holds entries', async () => {
    await submitted();
    act(() => mockControls().fetchError('provider_failure', 'Download failed.'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-fetch-again')).toBeTruthy());
    act(() => mockControls().spool(true));
    fireEvent.click(screen.getByTestId('rgn-job-fetch-again'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').className).toMatch(/error/),
    );
    expect(screen.getByTestId('rgn-job-fetch-error')).toBeTruthy();
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
  });

  test('FR-RGN-28 R109 Dismiss after a failed fetch asks first, then ends the job (failed, fetch_failed in core)', async () => {
    await submitted();
    act(() => mockControls().fetchError('provider_failure', 'Download failed.'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-fetch-error')).toBeTruthy());
    // Cancel keeps it as it was.
    fireEvent.click(screen.getByTestId('rgn-job-dismiss'));
    expect(screen.getByTestId('rgn-job-dismiss-ask').textContent).toMatch(
      /You may have paid for this; its results can’t be downloaded/,
    );
    fireEvent.click(screen.getByTestId('rgn-job-dismiss-cancel'));
    expect(screen.queryByTestId('rgn-job-dismiss-ask')).toBeNull();
    expect(fake.callsOf('rgn_job_dismiss')).toHaveLength(0);
    fireEvent.click(screen.getByTestId('rgn-job-dismiss'));
    fireEvent.click(screen.getByTestId('rgn-job-dismiss-confirm'));
    await waitFor(() => expect(fake.callsOf('rgn_job_dismiss')).toHaveLength(1));
    // Resolved with no iteration: it leaves the tracker (as a dismissed failed submit).
    await waitFor(() => expect(screen.queryByTestId('rgn-job-strip')).toBeNull());
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
  });

  test('FR-RGN-24 R108 after a reopen a fetch error and Fetch again come back from the row', async () => {
    await submitted();
    act(() => mockControls().fetchError('provider_failure', 'Download failed.'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-fetch-again')).toBeTruthy());
    // A reopen: the session's live state is gone, only the iterations row carries the error.
    const project = projectStore.get().project!;
    act(() => rgn.detach());
    await act(() => rgn.attach(project));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')).toHaveLength(1));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-job-fetch-error').dataset.code).toBe('provider_failure'),
    );
    expect(screen.getByTestId('rgn-job-fetch-again')).toBeTruthy();
  });

  test('FR-RGN-23 R108 a lost job without a handle says its result can’t be recovered, with the hint', async () => {
    await submitted();
    act(() =>
      mockControls().lose(false, 'timeout', 'The provider didn’t confirm the submit. Check it.'),
    );
    await waitFor(() => expect(screen.getByTestId('rgn-job-state').textContent).toBe('Lost'));
    const lost = screen.getByTestId('rgn-job-lost');
    expect(lost.dataset.handle).toBe('none');
    expect(lost.textContent).toMatch(/can’t be recovered/);
    expect(screen.getByTestId('rgn-job-charge-hint').textContent).toBe(
      'The provider didn’t confirm the submit. Check it.',
    );
  });

  test('FR-RGN-23 R108 a lost job with its handle kept may still finish at the provider', async () => {
    await submitted();
    act(() => mockControls().lose(true, 'auth'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-lost').dataset.handle).toBe('kept'));
    const lost = screen.getByTestId('rgn-job-lost');
    expect(lost.textContent).toMatch(/may still finish at the provider/);
    expect(lost.textContent).not.toMatch(/can’t be recovered/);
    // No hint from the core: the generic possible-charge hint.
    expect(screen.getByTestId('rgn-job-charge-hint').textContent).toMatch(
      /may have been charged; check the provider before retrying/,
    );
  });

  test('FR-RGN-21 R108 a lost job keeps Retry with the possible-charge hint next to it; Retry only pre-fills', async () => {
    await submitted({}, 'first prompt');
    act(() => mockControls().lose(true, 'auth', 'Polling stopped. Check the provider first.'));
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    const retry = screen.getByTestId('rgn-job-retry');
    const hint = screen.getByTestId('rgn-job-charge-hint');
    expect(hint.parentElement).toBe(retry.parentElement);
    expect(retry.nextElementSibling).toBe(hint);
    expect(hint.textContent).toBe('Polling stopped. Check the provider first.');
    fireEvent.click(retry);
    await waitFor(() =>
      expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).value).toBe(
        'first prompt',
      ),
    );
    expect(fake.callsOf('rgn_job_submit')).toHaveLength(1);
  });

  test('FR-RGN-21 retry pre-fills the panel from the job and requires a new estimate', async () => {
    await submitted({}, 'first prompt');
    act(() => mockControls().fail());
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    act(() => rgn.setPrompt('changed'));
    act(() => rgn.setMargin(32));
    const before = fake.callsOf('rgn_job_estimate').length;
    fireEvent.click(screen.getByTestId('rgn-job-retry'));
    await waitFor(() =>
      expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).value).toBe(
        'first prompt',
      ),
    );
    expect(rgn.get().margin).toBe(128);
    await waitFor(() => expect(fake.callsOf('rgn_job_estimate').length).toBe(before + 1), {
      timeout: 2000,
    });
  });

  test('FR-RGN-21 retry restores the model and the blend toggle off', async () => {
    await setup({ spend: { cap: 100, threshold: 10 } });
    await chooseOption(screen.getByTestId('rgn-panel-model'), 'Simple');
    await waitFor(() => expect(rgn.get().modelId).toBe('fake-simple'));
    fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
    expect(rgn.get().blendInstruction).toBe(false);
    await typePrompt('a stone well');
    await estimated();
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')).toHaveLength(1));
    expect(fake.callsOf('rgn_job_submit')[0]!.draft).toMatchObject({
      model_id: 'fake-simple',
      blend_instruction: false,
    });
    act(() => mockControls().fail());
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    // Changed since: the other model, blend on (the tracker is a listbox too: no select helper).
    await act(() => rgn.pickModel('fake-all-types'));
    await waitFor(() => expect(rgn.get().modelId).toBe('fake-all-types'));
    fireEvent.click(screen.getByTestId('rgn-panel-blend-instruction'));
    expect(rgn.get().blendInstruction).toBe(true);
    fireEvent.click(screen.getByTestId('rgn-job-retry'));
    await waitFor(() => expect(rgn.get().modelId).toBe('fake-simple'));
    expect(rgn.get().blendInstruction).toBe(false);
    expect(screen.getByTestId('rgn-panel-model').textContent).toContain('Simple');
    expect(screen.getByTestId('rgn-panel-blend-instruction').getAttribute('aria-checked')).toBe(
      'false',
    );
  });

  test('FR-RGN-21 retry restores the count the user set; FR-RGN-11 the draft keeps it', async () => {
    await setup();
    act(() => rgn.setValue('num_samples', 4));
    await typePrompt('four please');
    await estimated();
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')).toHaveLength(1));
    act(() => mockControls().fail());
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    act(() => rgn.setValue('num_samples', 1));
    fireEvent.click(screen.getByTestId('rgn-job-retry'));
    await waitFor(() => expect(rgn.get().values.num_samples).toBe(4));
    expect(rgn.get().touched).toContain('num_samples');
    // The draft written after the retry keeps it (FR-RGN-11).
    await waitFor(
      () => {
        const saved = fake.callsOf('project_put_draft').at(-1) as {
          draft: { controls: Record<string, unknown> };
        };
        expect(saved.draft.controls.num_samples).toBe(4);
      },
      { timeout: 4000 },
    );
  });

  test('FR-RGN-11 a restored count a single-output model rejects is dropped with a notice', async () => {
    const all = structuredClone(ALL_TYPES);
    await submitted({ capabilities: { 'fake-all-types': all, 'fake-simple': SIMPLE } });
    act(() => mockControls().fail());
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    all.controls = all.controls.filter((c) => c.role !== 'candidate_count');
    fireEvent.click(screen.getByTestId('rgn-job-retry'));
    await waitFor(() =>
      expect(rgn.get().notices.some((n) => n.includes('num samples'))).toBe(true),
    );
    expect(rgn.get().values.num_samples).toBeUndefined();
    expect(screen.getByTestId('rgn-panel-count-single')).toBeTruthy();
  });

  test('FR-RGN-40 tracker: iterations newest first with kept/returned, cost, ✓; the picker shows the selected one', async () => {
    await submitted();
    act(() => mockControls().finish(3, [2]));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
    const chip = screen.getAllByTestId('rgn-tracker-item')[0]!;
    expect(chip.dataset.n).toBe('1');
    expect(chip.textContent).toContain('001');
    expect(chip.textContent).toContain('turn this room into a kitchen');
    expect(screen.getAllByTestId('edt-picker-item').map((e) => e.dataset.status)).toEqual([
      'ready',
      'ready',
      'invalid',
    ]);
    // Thumbnails come from edt_image_open with max_side 256 (FR-EDT-32).
    await waitFor(() => expect(fake.callsOf('edt_image_open').length).toBe(2));
    expect(fake.callsOf('edt_image_open')[0]).toMatchObject({ max_side: 256 });

    // onSelect → preview → compare; blend change re-previews.
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(1));
    expect(screen.getByTestId('rgn-panel-blend')).toBeTruthy();
    fireEvent.change(screen.getByTestId('rgn-blend-feather'), { target: { value: '96' } });
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(2));
    expect(fake.callsOf('rgn_candidate_preview')[1]!.blend).toEqual({
      feather_px: 96,
      colour_match: true,
    });

    // onStar #2, onAccept #1.
    fireEvent.click(screen.getAllByTestId('edt-picker-star')[1]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_star')).toHaveLength(1));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    await waitFor(() => expect(fake.callsOf('rgn_candidate_accept')).toHaveLength(1));
    expect(fake.callsOf('rgn_candidate_accept')[0]!.blend).toEqual({
      feather_px: 96,
      colour_match: true,
    });
    await waitFor(() =>
      expect(screen.getAllByTestId('rgn-tracker-item')[0]!.dataset.accepted).toBe('true'),
    );
    expect(screen.getByTestId('rgn-tracker-accepted')).toBeTruthy();
    // Kept: #1 accepted + #2 starred; #3 invalid. Prompt cleared after accept.
    expect(screen.getByTestId('rgn-tracker-kept').textContent).toBe('2/3 kept');
    expect((screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement).value).toBe('');
    expect(screen.queryByTestId('rgn-job-strip')).toBeNull();

    // A second job: it leads; the older iteration stays selectable, its accept is refused.
    await typePrompt('second idea');
    await estimated();
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')).toHaveLength(2));
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getAllByTestId('rgn-tracker-item')[0]!.dataset.n).toBe('2'));
    const items = screen.getAllByTestId('rgn-tracker-item');
    expect(items[0]!.getAttribute('aria-selected')).toBe('true');
    fireEvent.click(items[1]!);
    // 001 keeps its accepted #1, starred #2 and the invalid #3 (nothing of it is trashed).
    await waitFor(() =>
      expect(screen.getAllByTestId('edt-picker-item').map((e) => e.dataset.recordId)).toEqual(
        rgn
          .get()
          .candidates.filter((c) => c.job_id === items[1]!.dataset.recordId)
          .map((c) => c.id),
      ),
    );
    expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3);
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview').length).toBe(3));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
        /Only the newest iteration/,
      ),
    );
  });

  test('FR-RGN-29 re-tune the accepted patch with new blend values', async () => {
    await submitted();
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(1));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    await waitFor(() => expect(screen.getByTestId('rgn-blend-retune')).toBeTruthy());
    fireEvent.change(screen.getByTestId('rgn-blend-feather'), { target: { value: '32' } });
    fireEvent.click(screen.getByTestId('rgn-blend-colour'));
    fireEvent.click(screen.getByTestId('rgn-blend-retune'));
    await waitFor(() => expect(fake.callsOf('rgn_patch_retune')).toHaveLength(1));
    expect(fake.callsOf('rgn_patch_retune')[0]!.blend).toEqual({
      feather_px: 32,
      colour_match: false,
    });
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toBe('Patch re-tuned.×'),
    );
  });

  test('FR-RGN-30 a double accept is one accept; quick re-tunes each take the previous result', async () => {
    await submitted();
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(1));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    await waitFor(() => expect(screen.getByTestId('rgn-blend-retune')).toBeTruthy());
    await wait(300);
    expect(fake.callsOf('rgn_candidate_accept')).toHaveLength(1);
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toBe('Patch accepted.×');

    const retuneResults: string[] = [];
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const retune = handlers.rgn_patch_retune!;
    handlers.rgn_patch_retune = (i) => {
      const r = retune(i) as { layer_id: string };
      retuneResults.push(r.layer_id);
      return r;
    };
    fireEvent.click(screen.getByTestId('rgn-blend-retune'));
    fireEvent.click(screen.getByTestId('rgn-blend-retune'));
    await waitFor(() => expect(fake.callsOf('rgn_patch_retune')).toHaveLength(2));
    // The second re-tunes the first's layer (so its undo parent is the first's revision).
    expect(fake.callsOf('rgn_patch_retune')[1]!.layer_id).toBe(retuneResults[0]);
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toBe('Patch re-tuned.×'),
    );
  });

  test('FR-RGN-30 night review 12b a second accept of another candidate while one runs is refused visibly', async () => {
    await submitted();
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    const [a, b] = rgn.get().candidates.map((c) => c.id);
    // The first accept stays in flight until the gate opens.
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const accept = handlers.rgn_candidate_accept!;
    let open!: () => void;
    const gate = new Promise<void>((r) => (open = r));
    handlers.rgn_candidate_accept = async (i) => {
      await gate;
      return accept(i);
    };
    let first!: Promise<void>;
    act(() => {
      first = rgn.accept(a!);
    });
    await act(() => rgn.accept(b!));
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
      /Another patch is still being accepted/,
    );
    open();
    await act(() => first);
    expect(fake.callsOf('rgn_candidate_accept').map((c) => c.candidate_id)).toEqual([a]);
    // The first's notice keeps saying the second wasn't done.
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
      /^Patch accepted\. The other candidate wasn’t accepted/,
    );
  });

  test('FR-RGN-26 a preview without enough context says colour match was skipped', async () => {
    await submitted();
    const handlers = (
      window as unknown as { __pfMockHandlers: Record<string, (i: unknown) => unknown> }
    ).__pfMockHandlers;
    const preview = handlers.rgn_candidate_preview!;
    handlers.rgn_candidate_preview = (i) => ({
      ...(preview(i) as object),
      notices: ['colour_match_skipped'],
    });
    act(() => mockControls().finish(2));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(/Colour match skipped/),
    );
  });

  test('FR-RGN-28 owner #56 dismiss keeps every candidate', async () => {
    await submitted();
    act(() => mockControls().finish(3));
    await waitFor(() => expect(screen.getByTestId('rgn-job-dismiss')).toBeTruthy());
    fireEvent.click(screen.getByTestId('rgn-job-dismiss'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
        /Its candidates are kept/,
      ),
    );
    expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3);
    expect(rgn.get().candidates.every((c) => c.status === 'dismissed' && !c.trash_id)).toBe(true);
  });

  test('owner #56 X discards the selected candidate and the selection moves on', async () => {
    await submitted();
    act(() => mockControls().finish(3));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
    const ids = screen.getAllByTestId('edt-picker-item').map((e) => e.dataset.recordId!);
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(rgn.get().selectedCandidateId).toBe(ids[0]));
    fireEvent.keyDown(window, { key: 'x' });
    await waitFor(() => expect(fake.callsOf('rgn_candidate_discard')).toHaveLength(1));
    expect(fake.callsOf('rgn_candidate_discard')[0]!.candidate_id).toBe(ids[0]);
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    expect(rgn.get().selectedCandidateId).toBe(ids[1]);
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
      /in the trash for 30 days/,
    );
  });

  test('owner #56 Delete on a focused candidate discards it, not the region; focus moves on', async () => {
    await submitted();
    act(() => mockControls().finish(3));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
    const items = screen.getAllByTestId('edt-picker-item');
    const ids = items.map((e) => e.dataset.recordId!);
    const inner = rgn.get().inner;
    items[1]!.focus();
    fireEvent.keyDown(items[1]!, { key: 'Delete' });
    await waitFor(() => expect(fake.callsOf('rgn_candidate_discard')).toHaveLength(1));
    expect(fake.callsOf('rgn_candidate_discard')[0]!.candidate_id).toBe(ids[1]);
    expect(rgn.get().inner).toEqual(inner);
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    await waitFor(() =>
      expect(document.activeElement?.getAttribute('data-record-id')).toBe(ids[2]),
    );
  });

  test('owner #56 Discard the rest after an accept: one call, accepted and starred stay', async () => {
    await submitted();
    act(() => mockControls().finish(4));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(4));
    expect(screen.queryByTestId('edt-picker-discard-rest')).toBeNull();
    fireEvent.click(screen.getAllByTestId('edt-picker-star')[1]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_star')).toHaveLength(1));
    fireEvent.click(screen.getAllByTestId('edt-picker-item')[0]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_preview')).toHaveLength(1));
    fireEvent.click(screen.getByTestId('edt-picker-accept'));
    await waitFor(() => expect(fake.callsOf('rgn_candidate_accept')).toHaveLength(1));
    // Accept keeps the rest (#56); the accepted one can't be discarded.
    await waitFor(() => expect(screen.getByTestId('edt-picker-discard-rest')).toBeTruthy());
    expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(4);
    expect((screen.getAllByTestId('edt-picker-discard')[0] as HTMLButtonElement).disabled).toBe(
      true,
    );
    // Walk owner-UX L4: Radix tips, not native titles.
    const discard0 = screen.getAllByTestId('edt-picker-discard')[0]!;
    expect(discard0.getAttribute('title')).toBeNull();
    expect(discard0.closest('[data-tooltip]')?.getAttribute('data-tooltip')).toBe(
      'The accepted candidate can’t be discarded',
    );
    expect(screen.getByTestId('edt-picker-discard-rest').className).toContain(
      'edt-picker-discard-rest',
    );
    fireEvent.click(screen.getByTestId('edt-picker-discard-rest'));
    await waitFor(() => expect(fake.callsOf('rgn_job_discard_rest')).toHaveLength(1));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(2));
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(/2 candidates discarded/);
    expect(screen.queryByTestId('edt-picker-discard-rest')).toBeNull();
  });

  test('FR-RGN-46 #61 Discard all bins the set (starred too), no confirm; Undo restores it with the star', async () => {
    await submitted();
    // Disabled while the job runs (no result set yet: no button at all).
    expect(screen.queryByTestId('rgn-tracker-discard-all')).toBeNull();
    act(() => mockControls().finish(3));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
    fireEvent.click(screen.getAllByTestId('edt-picker-star')[1]!);
    await waitFor(() => expect(fake.callsOf('rgn_candidate_star')).toHaveLength(1));
    const all = () => screen.getByTestId('rgn-tracker-discard-all') as HTMLButtonElement;
    await waitFor(() => expect(all().disabled).toBe(false));
    fireEvent.click(all());
    await waitFor(() => expect(fake.callsOf('rgn_iteration_discard_all')).toHaveLength(1));
    expect(screen.queryByTestId('rgn-confirm-ok')).toBeNull();
    await waitFor(() => expect(screen.queryAllByTestId('edt-picker-item')).toHaveLength(0));
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
      /3 results in the trash for 30 days/,
    );
    await waitFor(() => expect(all().disabled).toBe(true));
    fireEvent.click(screen.getByTestId('rgn-tracker-discard-undo'));
    await waitFor(() => expect(fake.callsOf('rgn_iteration_discard_all_undo')).toHaveLength(1));
    await waitFor(() => expect(screen.getAllByTestId('edt-picker-item')).toHaveLength(3));
    expect(screen.getAllByTestId('edt-picker-star')[1]!.getAttribute('aria-pressed')).toBe('true');
    expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(/3 results restored/);
    expect(all().disabled).toBe(false);
    // walk-61 L2: a later Discard all (here from Results) replaces the restored notice.
    act(() =>
      projectStore.setTrashNotice({ at: 'results', text: '3 results in the trash for 30 days' }),
    );
    expect(screen.queryByTestId('rgn-tracker-notice')).toBeNull();
    // And a second Discard all here shows only its own notice.
    fireEvent.click(all());
    await waitFor(() => expect(fake.callsOf('rgn_iteration_discard_all')).toHaveLength(2));
    await waitFor(() =>
      expect(screen.getAllByTestId('rgn-tracker-notice').map((n) => n.textContent)).toEqual([
        expect.stringMatching(/^3 results in the trash for 30 days/),
      ]),
    );
  });
});

test('FR-RGN-07 a late model list of the previous project never lands in the next one', async () => {
  polyfillRadix();
  fake = mockCore();
  const w = window as unknown as {
    __pfMockHandlers: Record<string, (i: unknown) => unknown>;
    __pfMockSnapshot: Record<string, unknown>;
  };
  const list = w.__pfMockHandlers.provider_list_models!;
  let release: () => void = () => undefined;
  let first = true;
  w.__pfMockHandlers.provider_list_models = (i) => {
    if (!first) return list(i);
    first = false;
    // Project A's answer comes late.
    return new Promise<void>((r) => (release = r)).then(() => list(i));
  };
  w.__pfMockSnapshot.draft = { model_id: 'fake-all-types', controls: { guidance: 15 } };
  projectStore.reset();
  await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
  render(
    <Tooltip.Provider>
      <GeneratePanel />
    </Tooltip.Provider>,
  );
  await waitFor(() => expect(rgn.get().modelsLoading).toBe(true));
  // Project B (no draft) opens and loads its own list.
  w.__pfMockSnapshot.project_id = '01JPROJECTB000000000000000';
  w.__pfMockSnapshot.draft = null;
  await act(() => projectStore.open('C:/pf-test/library/Maps/other'));
  await waitFor(() => expect(rgn.get().projectId).toBe('01JPROJECTB000000000000000'));
  await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
  expect(rgn.get().values.guidance).toBe(7.5);
  await act(async () => release());
  await wait(100);
  expect(rgn.get().values.guidance).toBe(7.5);
  expect(rgn.get().touched).not.toContain('guidance');
});

// A draft from before its model left the edit list (2026-11-11): GPT Image 2 left the overlay;
// GPT Image 2.5 kept only its generate rows (paid masked-edit check failed).
test.each([
  ['GPT Image 2, removed', 'model_openai-gpt-image-2'],
  ['GPT Image 2.5 Flare, generate only', 'model_openai-gpt-image-2-5-flare'],
])(
  'FR-RGN-11 a saved model the list no longer offers falls back to the first one, with a notice (%s)',
  async (_label, gone) => {
    polyfillRadix();
    fake = mockCore();
    const w = window as unknown as { __pfMockSnapshot: Record<string, unknown> };
    w.__pfMockSnapshot.draft = { model_id: gone, controls: { guidance: 15 } };
    projectStore.reset();
    await act(() => projectStore.open('C:/pf-test/library/Maps/map'));
    render(
      <Tooltip.Provider>
        <GeneratePanel />
      </Tooltip.Provider>,
    );
    await waitFor(() => expect(rgn.get().evaluation).not.toBeNull());
    const first = (await api.provider.listModels('edit_masked')).models[0]!.id;
    expect(rgn.get().modelId).toBe(first);
    expect(rgn.get().notices).toContain(
      'The saved model isn’t available; the first one is selected.',
    );
    // The gone model's remembered values don't carry over.
    expect(rgn.get().values.guidance).toBe(7.5);
    expect(fake.callsOf('provider_get_capabilities').map((c) => c.model_id)).not.toContain(gone);
  },
);
