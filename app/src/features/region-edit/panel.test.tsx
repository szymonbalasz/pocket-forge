import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, test } from 'vitest';
import { Tooltip } from 'radix-ui';

import type { FakeTauri } from '../../api/mocks/tauri';
import type { RgnMockSeed } from '../../api/mocks/rgn';
import { projectStore } from '../../store/project';
import { editorFeature } from '../editor';
import { GeneratePanel } from './GeneratePanel';
import { IterationTracker } from './IterationTracker';
import { JobStatus } from './JobStatus';
import { rgn } from './store';
import { mockControls, mockCore, polyfillRadix } from './testUtils';

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
  fireEvent.change(screen.getByTestId('rgn-panel-prompt'), { target: { value: text } });
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
    expect(screen.getByTestId('rgn-panel-prompt')).toBeTruthy();
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

  test('FR-RGN-09 the final prompt is sent byte for byte; submit disabled while empty', async () => {
    await setup({ spend: { cap: 100, threshold: 10 } });
    await estimated();
    expect(screen.getByTestId('rgn-panel-submit-reason').textContent).toBe('Required: prompt.');
    const prompt = '  Turn this room into a kitchen,\nwith copper pots  ';
    await typePrompt(prompt);
    expect(screen.getByTestId('rgn-panel-final-prompt').textContent).toBe(prompt);
    expect((screen.getByTestId('rgn-panel-submit') as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByTestId('rgn-panel-submit'));
    fireEvent.click(await screen.findByTestId('rgn-confirm-ok'));
    await waitFor(() => expect(fake.callsOf('rgn_job_submit')).toHaveLength(1));
    expect(
      (fake.callsOf('rgn_job_submit')[0]!.draft as { final_prompt: string }).final_prompt,
    ).toBe(prompt);
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

  test('FR-RGN-21 retry pre-fills the panel from the job and requires a new estimate', async () => {
    await submitted({}, 'first prompt');
    act(() => mockControls().fail());
    await waitFor(() => expect(screen.getByTestId('rgn-job-retry')).toBeTruthy());
    act(() => rgn.setPrompt('changed'));
    act(() => rgn.setMargin(32));
    const before = fake.callsOf('rgn_job_estimate').length;
    fireEvent.click(screen.getByTestId('rgn-job-retry'));
    await waitFor(() =>
      expect((screen.getByTestId('rgn-panel-prompt') as HTMLTextAreaElement).value).toBe(
        'first prompt',
      ),
    );
    expect(rgn.get().margin).toBe(128);
    await waitFor(() => expect(fake.callsOf('rgn_job_estimate').length).toBe(before + 1), {
      timeout: 2000,
    });
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
    expect((screen.getByTestId('rgn-panel-prompt') as HTMLTextAreaElement).value).toBe('');
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

  test('FR-RGN-28 dismiss reports the trashed count', async () => {
    await submitted();
    act(() => mockControls().finish(3));
    await waitFor(() => expect(screen.getByTestId('rgn-job-dismiss')).toBeTruthy());
    fireEvent.click(screen.getByTestId('rgn-job-dismiss'));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-tracker-notice').textContent).toMatch(
        /3 candidates moved to the trash/,
      ),
    );
  });
});
