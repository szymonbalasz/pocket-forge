// #55: the blend instruction section of the settings dialog (mocked core).
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'vitest';

import type { FakeTauri } from '../../api/mocks/tauri';
import { settingsSections } from '../../shared/settings';
import { BlendInstructionSettings } from './BlendInstructionSettings';
import './index';
import { mockCore } from './testUtils';

let fake: FakeTauri;

afterEach(() => {
  cleanup();
  fake.uninstall();
});

describe('Blend instruction settings (#55)', () => {
  test('owner-55 settings text: shows the default, saves an edit, resets to default', async () => {
    fake = mockCore();
    expect(settingsSections().map((s) => s.id)).toContain('rgn-blend-instruction');
    // Walk owner-UX L4: the section is headed by its feature, beside "Scenario provider".
    expect(settingsSections().find((s) => s.id === 'rgn-blend-instruction')?.title).toBe(
      'Region edit',
    );
    render(<BlendInstructionSettings />);
    const text = screen.getByTestId('rgn-settings-blend-text') as HTMLTextAreaElement;
    await waitFor(() => expect(text.value).toMatch(/^Keep everything near the edges/));
    const save = screen.getByTestId('rgn-settings-blend-save') as HTMLButtonElement;
    const reset = screen.getByTestId('rgn-settings-blend-reset') as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(reset.disabled).toBe(true);

    fireEvent.change(text, { target: { value: 'Match the ink.' } });
    await act(async () => fireEvent.click(save));
    await waitFor(() =>
      expect(screen.getByTestId('rgn-settings-blend-result').textContent).toBe('Saved.'),
    );
    expect(fake.callsOf('rgn_blend_instruction_set').at(-1)).toEqual({ text: 'Match the ink.' });

    await act(async () => fireEvent.click(reset));
    await waitFor(() => expect(text.value).toMatch(/^Keep everything near the edges/));
    expect(fake.callsOf('rgn_blend_instruction_set').at(-1)).toEqual({ text: null });
  });
});
