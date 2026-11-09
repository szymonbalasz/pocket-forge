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
import { installChatSink } from './chatSink';
import { chatContext } from '../../shared/chatContext';
import { projectStore } from '../../store/project';
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
  // L6: the header chip is short; the message shows once, in the body.
  expect(id('prm-chat-status').textContent).toBe('Not logged in');
  await api.chat.fakeSetCase('success');
  fireEvent.click(id('prm-chat-recheck'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect(fake.callsOf('prm_chat_status').some((c) => c.refresh === true)).toBe(true);
});

test('FR-PRM-45 L2 the window Log in… says what happened; the fake logs in, Recheck continues', async () => {
  setup({ chat: { consent: true, case: 'not_logged_in' } });
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-login')).toBeTruthy());
  fireEvent.click(id('prm-chat-login'));
  await waitFor(() =>
    expect(id('prm-chat-login-note').textContent).toContain('logged in without a console'),
  );
  fireEvent.click(id('prm-chat-recheck'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
});

test('FR-PRM-20 L6 the window follows Settings → off without a refocus', async () => {
  setup({ chat: { consent: true } });
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  await act(() => api.chat.settingsPut({ enabled: false }));
  await waitFor(() => expect(screen.getByText('Prompt chat is off.')).toBeTruthy());
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
  // L6: the proposal shows once, as its card, not also as a fence in the reply.
  expect(document.querySelectorAll('.prm-chat-code')).toHaveLength(1);
  expect(input().value).toBe('');
  // FR-PRM-38 (L4): the fake's cost is notional.
  expect(id('prm-chat-usage').textContent).toBe('≈ $0.00 notional (fake chat, nothing billed)');

  fireEvent.click(id('prm-chat-proposal-copy'));
  await waitFor(() => expect(id('prm-chat-proposal-copy').textContent).toBe('Copied'));
  expect(writeText).toHaveBeenCalledWith('make this a kitchen');

  fireEvent.click(id('prm-chat-proposal-replace'));
  await waitFor(() => expect(screen.getByText(/^Inserted /)).toBeTruthy());
  expect(fake.callsOf('prm_chat_insert')).toHaveLength(1);
});

test('FR-PRM-22 M3 the transcript shows the newest turn on open and after a send', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  const set: number[] = [];
  let height = 300;
  const proto = HTMLOListElement.prototype;
  Object.defineProperty(proto, 'scrollHeight', { configurable: true, get: () => height });
  Object.defineProperty(proto, 'clientHeight', { configurable: true, get: () => 100 });
  Object.defineProperty(proto, 'scrollTop', {
    configurable: true,
    get: () => set.at(-1) ?? 0,
    set: (v: number) => void set.push(v),
  });
  try {
    render(<ChatWindow />);
    await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
    expect(set.at(-1)).toBe(300);
    height = 900;
    await type('ideas');
    fireEvent.click(id('prm-chat-send'));
    await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
    expect(set.at(-1)).toBe(900);
  } finally {
    for (const k of ['scrollHeight', 'clientHeight', 'scrollTop'])
      delete (proto as unknown as Record<string, unknown>)[k];
  }
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
  // FR-PRM-25/26 (M4): the turn names the images the turn limit dropped.
  expect(id('prm-chat-dropped').textContent).toBe(
    'Not sent (turn limit): teal.png, yellow.png, red.png',
  );
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

test('FR-PRM-27 a double-clicked Retry sends once; Retry waits while a turn is sent', async () => {
  setup({ chat: { consent: true, case: 'usage_limit' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(id('prm-chat-retry')).toBeTruthy());
  await api.chat.fakeSetCase('slow(4)');
  const retry = id('prm-chat-retry');
  fireEvent.click(retry);
  fireEvent.click(retry);
  await waitFor(() => expect(id('prm-chat-stop')).toBeTruthy());
  expect(fake.callsOf('prm_chat_send')).toHaveLength(2);
  expect(screen.queryByTestId('prm-chat-error')).toBeNull();
  // The old failed turn's Retry is disabled while the new one runs.
  expect((id('prm-chat-retry') as HTMLButtonElement).disabled).toBe(true);
});

test('FR-PRM-37 L3 a crashed turn shows the stderr tail under Details', async () => {
  setup({ chat: { consent: true, case: 'crash' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText(/stopped unexpectedly/)).toBeTruthy());
  expect(id('prm-chat-error-detail').textContent).toContain('fake chat: simulated crash');
});

test('§7 §11 M5 an offline turn locks Send and Retry until Recheck succeeds', async () => {
  setup({ chat: { consent: true, case: 'offline' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText("Can't reach Anthropic.")).toBeTruthy());
  expect(screen.getByText(/Offline: Recheck once the connection is back/)).toBeTruthy();
  expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(true);
  expect((id('prm-chat-retry') as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(id('prm-chat-retry'));
  expect(fake.callsOf('prm_chat_send')).toHaveLength(1);
  // The connection is back: Recheck lifts the lock, then Retry resends.
  await api.chat.fakeSetCase('success');
  fireEvent.click(id('prm-chat-recheck'));
  await waitFor(() => expect((id('prm-chat-retry') as HTMLButtonElement).disabled).toBe(false));
  expect(screen.queryByTestId('prm-chat-recheck')).toBeNull();
  fireEvent.click(id('prm-chat-retry'));
  await waitFor(() => expect(fake.callsOf('prm_chat_send')).toHaveLength(2));
});

test('§7 R94 read-only project: the turns show, marked "Not saved"; Replace is disabled', async () => {
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
  expect(screen.getAllByText('ideas').length).toBeGreaterThan(0);
  expect(screen.getAllByTestId('prm-chat-unsaved').map((e) => e.textContent)).toEqual([
    'Not saved',
    'Not saved',
  ]);
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

test('FR-PRM-45 L2 Settings Log in… / Log out / Recheck work on the fake and the status follows', async () => {
  setup({ chat: { consent: true } });
  render(<ChatSettings />);
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Fake backend'));
  fireEvent.click(id('prm-settings-chat-logout'));
  await waitFor(() =>
    expect(id('prm-settings-chat-status').textContent).toContain("isn't logged in"),
  );
  fireEvent.click(id('prm-settings-chat-login'));
  await waitFor(() => expect(screen.getByText(/logged in without a console/)).toBeTruthy());
  fireEvent.click(id('prm-settings-chat-recheck'));
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Fake backend'));
  // A change made elsewhere (the chat window, a case switch) shows without a reload.
  await act(() => api.chat.fakeSetCase('offline'));
  await act(() => api.chat.settingsPut({ enabled: false }));
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Off.'));
});

test('FR-PRM-20 Settings before consent says so and offers no login', async () => {
  setup({});
  render(<ChatSettings />);
  await waitFor(() => expect(id('prm-settings-chat-status').textContent).toContain('Not opted in'));
  await act(async () => undefined);
  expect(screen.queryByTestId('prm-settings-chat-logout')).toBeNull();
});

test('BL-41 the window follows the main window by event, without polling', async () => {
  setup({ chat: { consent: true } });
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('No prompt block'));
  const reads = fake.callsOf('project_get').length;
  await api.chat.contextPublish(PID, 'gen', {});
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('New image'));
  await api.chat.contextClear(PID, 'gen');
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('No prompt block'));
  // The main window closed the project.
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_get = () => null;
  act(() => fake.emit('prm:chat-context', { project_id: null, host: null }));
  await waitFor(() => expect(id('prm-chat-no-project')).toBeTruthy());
  await new Promise((r) => setTimeout(r, 1200));
  expect(fake.callsOf('project_get').length).toBe(reads + 1);
});

test('BL-42 FR-PRM-26 the strip previews what the next turn sends', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-preview').textContent).toContain('crop 556×456 + mask'));
  expect(screen.getByAltText('region crop with the mask')).toBeTruthy();
  fireEvent.click(id('prm-chat-include-images'));
  await waitFor(() => expect(id('prm-chat-preview').textContent).toContain('text only'));
  expect(screen.queryByAltText('region crop with the mask')).toBeNull();
  expect(fake.callsOf('prm_chat_preview').at(-1)).toEqual({
    project_id: PID,
    include_images: false,
  });
});

test("BL-42 FR-PRM-23 a send from the window first flushes the main window's pending edit", async () => {
  setup({ chat: { consent: true } });
  chatContext.reset();
  installChatSink();
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_open = () => baseSnapshot();
  projectStore.reset();
  await act(() => projectStore.open(baseSnapshot().project_dir));
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'old' });
  render(<ChatWindow />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  // An edit in the main window, still inside its 300 ms debounce.
  chatContext.publish('rgn', { final_prompt: 'edited' });
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(fake.callsOf('prm_chat_send')).toHaveLength(1));
  const order = fake.calls.map((c) => c.cmd);
  const lastPublish = order.lastIndexOf('prm_context_publish');
  expect(lastPublish).toBeGreaterThan(-1);
  expect(lastPublish).toBeLessThan(order.indexOf('prm_chat_send'));
  expect(fake.callsOf('prm_context_publish').at(-1)!.input).toEqual({ final_prompt: 'edited' });
  chatContext.reset();
});
