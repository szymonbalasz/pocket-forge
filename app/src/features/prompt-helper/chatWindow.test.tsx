// The chat window and Settings → Prompt chat on the mocked core (prompt-helper §7, slice 6):
// consent, the backend error state with Recheck, send with streaming, proposal Replace + Copy,
// Stop, errors with Retry, no project, no prompt block, read-only, and the Chat button.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';

import { api, type ProjectSnapshot } from '../../api';
import { baseSnapshot, type FakeTauri } from '../../api/mocks/tauri';
import type { PrmMockSeed } from '../../api/mocks/prm';
import { ChatSettings } from './ChatSettings';
import { ChatWindow } from './ChatWindow';
import { mockPrm, TestHost } from './testUtils';

let fake: FakeTauri;
const PID = baseSnapshot().project_id;

afterEach(() => {
  cleanup();
  fake.uninstall();
  vi.restoreAllMocks();
});

function setup(seed: PrmMockSeed, project: ProjectSnapshot | null = baseSnapshot()): void {
  fake = mockPrm(seed);
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_get = () => project;
}

const id = (t: string) => screen.getByTestId(t);
const input = () => id('prm-chat-input') as HTMLTextAreaElement;

async function type(text: string) {
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  fireEvent.change(input(), { target: { value: text } });
}

test('FR-PRM-20 the notice comes first; Not now starts nothing, Accept opens the chat', async () => {
  setup({});
  render(<ChatWindow />);
  await waitFor(() => expect(screen.getByText(/Prompt chat sends data to Anthropic/)).toBeTruthy());
  fireEvent.click(id('prm-chat-consent-decline'));
  expect(screen.getByText(/The chat stays off/)).toBeTruthy();
  expect(fake.callsOf('prm_chat_consent')).toHaveLength(0);
  fireEvent.click(id('prm-chat-consent-accept'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect(fake.callsOf('prm_chat_consent')).toEqual([{ accept: true }]);
  expect(id('prm-chat-fake').textContent).toBe('Fake chat');
  expect(id('prm-chat-status').textContent).toBe('Fake backend');
});

test('FR-PRM-33 FR-PRM-37 a backend error shows its message and fix; Recheck re-detects', async () => {
  setup({ chat: { consent: true, case: 'not_logged_in' } });
  render(<ChatWindow />);
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain("isn't logged in"));
  expect(screen.getByText('Log in…')).toBeTruthy();
  await api.chat.fakeSetCase('success');
  fireEvent.click(id('prm-chat-recheck'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect(fake.callsOf('prm_chat_status').some((c) => c.refresh === true)).toBe(true);
});

test('FR-PRM-21 no project open: "Open a project to chat about it"', async () => {
  setup({ chat: { consent: true } }, null);
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-no-project').textContent).toContain('Open a project'));
});

test('FR-PRM-27 without an attached prompt block Send is disabled with the reason', async () => {
  setup({ chat: { consent: true } });
  render(<ChatWindow />);
  await type('hello');
  expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText(/Open the Generate panel/)).toBeTruthy();
  expect(id('prm-chat-host').textContent).toBe('No prompt block');
});

test('FR-PRM-27 FR-PRM-28 FR-PRM-29 send streams the reply; Replace and Copy on the proposal', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  const writeText = vi.fn(() => Promise.resolve());
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('make this a kitchen');
  fireEvent.keyDown(input(), { key: 'Enter', ctrlKey: true });
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  expect(screen.getByText(/You wrote: "make this a kitchen"/)).toBeTruthy();
  expect(input().value).toBe('');
  expect(id('prm-chat-usage').textContent).toBe('≈ $0.00 billed');

  fireEvent.click(id('prm-chat-proposal-copy'));
  await waitFor(() => expect(id('prm-chat-proposal-copy').textContent).toBe('Copied'));
  expect(writeText).toHaveBeenCalledWith('make this a kitchen');

  fireEvent.click(id('prm-chat-proposal-replace'));
  await waitFor(() => expect(screen.getByText(/^Inserted /)).toBeTruthy());
  expect(fake.callsOf('prm_chat_insert')).toHaveLength(1);
});

test('FR-PRM-27 Stop keeps the partial reply', async () => {
  setup({ chat: { consent: true, case: 'slow(4)' } });
  await api.chat.contextPublish(PID, 'gen', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('New image'));
  await type('a crypt');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(id('prm-chat-stop')).toBeTruthy());
  fireEvent.click(id('prm-chat-stop'));
  await waitFor(() => expect(screen.getByText('Stopped.')).toBeTruthy());
  expect(screen.queryAllByTestId('prm-chat-proposal')).toHaveLength(0);
  expect(id('prm-chat-send')).toBeTruthy();
});

test('FR-PRM-37 a failed turn shows its error with Retry, which resends the same text', async () => {
  setup({ chat: { consent: true, case: 'usage_limit' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText(/usage limit is reached/)).toBeTruthy());
  expect(screen.getByText(/resets 3pm/)).toBeTruthy();
  await api.chat.fakeSetCase('success');
  fireEvent.click(screen.getByText('Retry'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  const sends = fake.callsOf('prm_chat_send');
  expect(sends.map((s) => s.text)).toEqual(['ideas', 'ideas']);
});

test('§7 read-only project: the chat works, nothing is saved, Replace is disabled', async () => {
  setup({ chat: { consent: true } }, {
    ...baseSnapshot(),
    read_only: 'original_changed',
  } as ProjectSnapshot);
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(screen.getByText('Not saved: read-only project.')).toBeTruthy());
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  expect((id('prm-chat-proposal-replace') as HTMLButtonElement).disabled).toBe(true);
});

test('FR-PRM-21 the Chat button opens the chat window', async () => {
  setup({});
  render(<TestHost host="rgn" prompt="x" />);
  fireEvent.click(id('prm-block-chat'));
  await waitFor(() => expect(fake.callsOf('prm_window_open')).toHaveLength(1));
});

test('FR-PRM-20 FR-PRM-45 Settings → Prompt chat: on/off, the status, the path override', async () => {
  setup({ chat: { consent: true } });
  render(<ChatSettings />);
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Fake backend'));
  expect((id('prm-settings-chat-toggle') as HTMLInputElement).checked).toBe(true);
  expect(screen.queryByTestId('prm-settings-chat-login')).toBeNull();
  fireEvent.click(id('prm-settings-chat-toggle'));
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Off.'));
  expect(fake.callsOf('prm_chat_settings_put')).toEqual([{ enabled: false }]);
  fireEvent.change(id('prm-settings-chat-path'), { target: { value: ' C:/tools/claude.exe ' } });
  fireEvent.blur(id('prm-settings-chat-path'));
  await waitFor(() =>
    expect(fake.callsOf('prm_chat_settings_put').at(-1)).toEqual({
      claude_path: 'C:/tools/claude.exe',
    }),
  );
  fireEvent.click(screen.getByText('Show privacy notice'));
  expect(screen.getByText(/Prompt chat sends data to Anthropic/)).toBeTruthy();
});

test('FR-PRM-20 Settings before consent says so and offers no login', async () => {
  setup({});
  render(<ChatSettings />);
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Not opted in'));
  await act(async () => undefined);
  expect(screen.queryByTestId('prm-settings-chat-logout')).toBeNull();
});
