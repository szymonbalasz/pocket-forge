// The chat panel and Settings → Prompt chat on the mocked core (prompt-helper §7): consent, the
// backend error state with Recheck, send with streaming, proposal Replace + Copy, Stop, errors
// with Retry, no project, no prompt block, read-only; the dock (#61: open, hide, resize,
// remembered) and the Chat button.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';

import { api, type ProjectSnapshot } from '../../api';
import { baseSnapshot, type FakeTauri } from '../../api/mocks/tauri';
import type { PrmMockSeed } from '../../api/mocks/prm';
import { ChatDock, ChatDockToggle } from './ChatDock';
import { ChatPanel } from './ChatPanel';
import { ChatSettings } from './ChatSettings';
import { installChatSink } from './chatSink';
import { resetDockStore } from './dock';
import { chatContext } from '../../shared/chatContext';
import { setDockRoom } from '../../shared/dockRoom';
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
  resetDockStore();
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
  render(<ChatPanel />);
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
  render(<ChatPanel />);
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
  render(<ChatPanel />);
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
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  await act(() => api.chat.settingsPut({ enabled: false }));
  await waitFor(() => expect(screen.getByText('Prompt chat is off.')).toBeTruthy());
});

test('FR-PRM-20 #61 "Turn it on" in the panel turns the chat on (enabled only)', async () => {
  setup({ chat: { consent: true } });
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  await act(() => api.chat.settingsPut({ enabled: false }));
  await waitFor(() => expect(screen.getByText('Prompt chat is off.')).toBeTruthy());
  fireEvent.click(id('prm-chat-enable'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect(fake.callsOf('prm_chat_settings_put')).toEqual([{ enabled: false }, { enabled: true }]);
  // No window-to-window event is involved any more.
  expect(fake.callsOf('plugin:event|emit')).toHaveLength(0);
});

test('FR-PRM-21 no project open: "Open a project to chat about it"', async () => {
  setup({ chat: { consent: true } }, null);
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-no-project').textContent).toContain('Open a project'));
});

test('FR-PRM-27 without an attached prompt block Send is disabled with the reason', async () => {
  setup({ chat: { consent: true } });
  render(<ChatPanel />);
  await type('hello');
  expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(true);
  expect(id('prm-chat-blocked-no-host').textContent).toMatch(/Open the Generate panel/);
  // R110: each reason has its own test id.
  expect(screen.queryByTestId('prm-chat-blocked-elsewhere')).toBeNull();
  expect(screen.queryByTestId('prm-chat-blocked-offline')).toBeNull();
  expect(id('prm-chat-host').textContent).toBe('No prompt block');
});

test('FR-PRM-27 FR-PRM-28 FR-PRM-29 send streams the reply; Replace and Copy on the proposal', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  const writeText = vi.fn(() => Promise.resolve());
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  render(<ChatPanel />);
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

test('FR-PRM-38 an api_key login: the strip warns once per thread and the footer says billed', async () => {
  setup({ chat: { consent: true } });
  type H = Record<string, (input: Record<string, unknown>) => unknown>;
  const h = (window as unknown as { __pfMockHandlers: H }).__pfMockHandlers;
  const status = h.prm_chat_status!;
  h.prm_chat_status = (i) => ({
    ...(status(i) as object),
    mode: 'claude-code',
    backend: {
      state: 'ok',
      version: '2.1.319',
      auth_method: 'api_key',
      plan: null,
      notional: false,
    },
  });
  // What the core records for a non-plan login (notional: false).
  const thread = h.prm_chat_thread_get!;
  h.prm_chat_thread_get = (i) => {
    const v = thread(i) as { thread: { turns: { usage?: { notional: boolean } }[] } | null };
    for (const t of v.thread?.turns ?? []) if (t.usage) t.usage.notional = false;
    return v;
  };
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  const warns = () => screen.queryAllByText('Billed to your Anthropic API key.');
  expect(warns()).toHaveLength(1);
  for (const text of ['one', 'two']) {
    await type(text);
    fireEvent.click(id('prm-chat-send'));
    await waitFor(() => expect(screen.getByText(new RegExp(`You wrote: "${text}"`))).toBeTruthy());
  }
  await waitFor(() => expect(id('prm-chat-usage').textContent).toBe('≈ $0.00 billed'));
  expect(warns()).toHaveLength(1);
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
    render(<ChatPanel />);
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
  render(<ChatPanel />);
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
  render(<ChatPanel />);
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
  render(<ChatPanel />);
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

test('FR-PRM-22 §11 an interrupted turn offers Resend; a double click sends once', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  // BL-43: the proposal can show while the first send still awaits its own thread reload. Wait
  // for that send to settle (Send is live again with text typed), so its late reload can't
  // overwrite the reopened thread below and Resend isn't disabled when clicked.
  await type('next');
  await waitFor(() => expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(false));
  // The app was killed mid-reply: on reopen the reply reads `interrupted` (§11, R103).
  const w = window as unknown as { __pfMockHandlers: Record<string, (i: never) => unknown> };
  const get = w.__pfMockHandlers.prm_chat_thread_get!;
  w.__pfMockHandlers.prm_chat_thread_get = (i: never) => {
    const v = get(i) as { thread: { turns: { state: string; text: string }[] } };
    const last = v.thread.turns.at(-1)!;
    Object.assign(last, { state: 'interrupted', text: '', proposals: [] });
    return v;
  };
  act(() => fake.emit('prm:chat-context', { project_id: PID, host: 'rgn' }));
  // A user can only click an enabled Resend.
  await waitFor(() => expect((id('prm-chat-resend') as HTMLButtonElement).disabled).toBe(false));
  w.__pfMockHandlers.prm_chat_thread_get = get;
  const resend = id('prm-chat-resend');
  fireEvent.click(resend);
  fireEvent.click(resend);
  await waitFor(() => expect(fake.callsOf('prm_chat_send')).toHaveLength(2));
  await act(async () => undefined);
  expect(fake.callsOf('prm_chat_send').map((s) => s.text)).toEqual(['ideas', 'ideas']);
});

test('FR-PRM-22 BL-43 an older thread reply that arrives late never overwrites a newer one', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  await type('next');
  await waitFor(() => expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(false));
  // Two reloads in a row; the first one's reply (the thread before the turn) arrives last.
  const w = window as unknown as { __pfMockHandlers: Record<string, (i: never) => unknown> };
  const get = w.__pfMockHandlers.prm_chat_thread_get!;
  let releaseOld!: () => void;
  let n = 0;
  w.__pfMockHandlers.prm_chat_thread_get = (i: never) => {
    const v = JSON.parse(JSON.stringify(get(i))) as { thread: { turns: unknown[] } };
    if (n++ > 0) return v;
    v.thread.turns = [];
    return new Promise((r) => (releaseOld = () => r(v)));
  };
  act(() => fake.emit('prm:chat-context', { project_id: PID, host: 'rgn' }));
  await waitFor(() => expect(n).toBe(1));
  act(() => fake.emit('prm:chat-context', { project_id: PID, host: 'rgn' }));
  await waitFor(() => expect(n).toBe(2));
  await act(async () => {
    releaseOld();
    await new Promise((r) => setTimeout(r, 0));
  });
  w.__pfMockHandlers.prm_chat_thread_get = get;
  expect(screen.getByText(/You wrote: "ideas"/)).toBeTruthy();
  expect(screen.queryByText(/Ask for ideas about the prompt/)).toBeNull();
});

test('FR-PRM-37 L3 a crashed turn shows the stderr tail under Details', async () => {
  setup({ chat: { consent: true, case: 'crash' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText(/stopped unexpectedly/)).toBeTruthy());
  expect(id('prm-chat-error-detail').textContent).toContain('fake chat: simulated crash');
});

test('§7 §11 M5 an offline turn locks Send and Retry until Recheck succeeds', async () => {
  setup({ chat: { consent: true, case: 'offline' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText("Can't reach Anthropic.")).toBeTruthy());
  expect(id('prm-chat-blocked-offline').textContent).toMatch(
    /Offline: Recheck once the connection is back/,
  );
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
  render(<ChatPanel />);
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

test('FR-PRM-21 a project switch in the main window switches the window to that thread', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('make this a kitchen');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getByText(/You wrote: "make this a kitchen"/)).toBeTruthy());
  // The main window opens project B: its (empty) thread, no host yet.
  const other = { ...baseSnapshot(), project_id: '01JPROJECTB000000000000000' };
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_get = () => other;
  act(() => fake.emit('prm:chat-context', { project_id: other.project_id, host: null }));
  await waitFor(() =>
    expect(fake.callsOf('prm_chat_thread_get').at(-1)?.project_id).toBe(other.project_id),
  );
  await waitFor(() => expect(screen.queryByText(/You wrote: "make this a kitchen"/)).toBeNull());
  expect(id('prm-chat-host').textContent).toBe('No prompt block');
  // Back to A: its thread again.
  w.__pfMockHandlers.project_get = () => baseSnapshot();
  act(() => fake.emit('prm:chat-context', { project_id: PID, host: 'rgn' }));
  await waitFor(() => expect(screen.getByText(/You wrote: "make this a kitchen"/)).toBeTruthy());
  expect(id('prm-chat-host').textContent).toBe('Region edit');
});

test('FR-PRM-27 night review 9 after a switch, the other project’s finishing turn disables Send with the reason', async () => {
  setup({ chat: { consent: true, case: 'slow(4)' } });
  const other = { ...baseSnapshot(), project_id: '01JPROJECTB000000000000000' };
  await api.chat.contextPublish(PID, 'rgn', {});
  await api.chat.contextPublish(other.project_id, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('a kitchen');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(id('prm-chat-stop')).toBeTruthy());
  // The main window opens B while A's reply still streams.
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_get = () => other;
  act(() => fake.emit('prm:chat-context', { project_id: other.project_id, host: 'rgn' }));
  await waitFor(() =>
    expect(fake.callsOf('prm_chat_thread_get').at(-1)?.project_id).toBe(other.project_id),
  );
  await type('a crypt');
  await waitFor(() =>
    expect(id('prm-chat-blocked-elsewhere').textContent).toMatch(/another project/),
  );
  expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(true);
  // A's turn ends: Send is live again for B.
  await waitFor(() => expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(false), {
    timeout: 5000,
  });
  expect(screen.queryByTestId('prm-chat-blocked-elsewhere')).toBeNull();
  expect(fake.callsOf('prm_chat_send')).toHaveLength(1);
});

test('FR-PRM-27 night review 9 a send refused with turn_active waits, saying why, not an error', async () => {
  setup({ chat: { consent: true, case: 'slow(4)' } });
  const other = { ...baseSnapshot(), project_id: '01JPROJECTB000000000000000' };
  await api.chat.contextPublish(PID, 'rgn', {});
  await api.chat.contextPublish(other.project_id, 'rgn', {});
  // A's turn started before this window knew of it; the window shows B.
  await api.chat.send(PID, 'a kitchen', false);
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_get = () => other;
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('a crypt');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() =>
    expect(id('prm-chat-blocked-elsewhere').textContent).toMatch(/another project/),
  );
  expect(screen.queryByTestId('prm-chat-error')).toBeNull();
  expect((id('prm-chat-send') as HTMLButtonElement).disabled).toBe(true);
  await waitFor(() => expect(screen.queryByTestId('prm-chat-blocked-elsewhere')).toBeNull(), {
    timeout: 5000,
  });
});

test('FR-PRM-22 §11 a locked thread file: the turns show "Not saved" with the banner', async () => {
  setup({ chat: { consent: true, locked: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  expect(screen.queryByTestId('prm-chat-not-saved')).toBeNull();
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  expect(id('prm-chat-not-saved').textContent).toContain('the chat file is locked');
  expect(screen.getAllByTestId('prm-chat-unsaved')).toHaveLength(2);
  // Not read-only: Replace still works.
  expect((id('prm-chat-proposal-replace') as HTMLButtonElement).disabled).toBe(false);
});

test('FR-PRM-21 #61 the Chat button opens the docked chat panel and remembers it', async () => {
  setup({ chat: { consent: true } });
  render(
    <>
      <TestHost host="rgn" prompt="x" />
      <ChatDock />
    </>,
  );
  await act(async () => undefined);
  expect(screen.queryByTestId('prm-dock')).toBeNull();
  fireEvent.click(id('prm-block-chat'));
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect((id('prm-dock') as HTMLElement).hidden).toBe(false);
  expect(fake.callsOf('prm_chat_settings_put')).toEqual([{ dock: { open: true } }]);
  // The startup read started nothing.
  expect(fake.callsOf('prm_chat_status')[0]).toEqual({ cached: true });
});

test('FR-PRM-21 #61 the dock opens where it was left, at its remembered width', async () => {
  setup({ chat: { consent: true, dockOpen: true } });
  await api.chat.settingsPut({ dock: { width: 560 } });
  render(<ChatDock />);
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  expect((id('prm-dock') as HTMLElement).style.width).toBe('560px');
});

test('FR-PRM-21 FR-PRM-35 #61 hiding the dock keeps a running reply; reopening shows it', async () => {
  setup({ chat: { consent: true, case: 'slow(5)' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(
    <>
      <ChatDock />
      <ChatDockToggle />
    </>,
  );
  fireEvent.click(id('prm-dock-toggle'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(id('prm-chat-stop')).toBeTruthy());
  fireEvent.click(id('prm-chat-close'));
  expect((id('prm-dock') as HTMLElement).hidden).toBe(true);
  expect(id('prm-dock-toggle').getAttribute('aria-pressed')).toBe('false');
  expect(fake.callsOf('prm_chat_cancel')).toHaveLength(0);
  expect(fake.callsOf('prm_chat_session_end')).toHaveLength(0);
  // The reply finishes while hidden; reopening shows it, streamed in full.
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1), {
    timeout: 5000,
  });
  fireEvent.click(id('prm-dock-toggle'));
  expect((id('prm-dock') as HTMLElement).hidden).toBe(false);
  expect(screen.getByText(/You wrote: "ideas"/)).toBeTruthy();
  expect(fake.callsOf('prm_chat_settings_put')).toEqual([
    { dock: { open: true } },
    { dock: { open: false } },
    { dock: { open: true } },
  ]);
});

test('FR-PRM-21 #61 the drag handle resizes within 320–720 px and saves the width once', async () => {
  setup({ chat: { consent: true, dockOpen: true } });
  render(<ChatDock />);
  await waitFor(() => expect(id('prm-dock')).toBeTruthy());
  const handle = id('prm-dock-resize');
  const dock = id('prm-dock') as HTMLElement;
  fireEvent.pointerDown(handle, { clientX: 1000, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientX: 900, pointerId: 1 });
  expect(dock.style.width).toBe('500px');
  fireEvent.pointerMove(handle, { clientX: 100, pointerId: 1 });
  expect(dock.style.width).toBe('720px');
  fireEvent.pointerMove(handle, { clientX: 1500, pointerId: 1 });
  expect(dock.style.width).toBe('320px');
  fireEvent.pointerUp(handle, { pointerId: 1 });
  expect(fake.callsOf('prm_chat_settings_put')).toEqual([{ dock: { width: 320 } }]);
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

test('FR-PRM-33 R96 Settings shows an ignored saved Claude path and clears it', async () => {
  setup({ chat: { consent: true, claudePath: 'tools/evil.exe' } });
  render(<ChatSettings />);
  await waitFor(() =>
    expect(id('prm-settings-chat-path-issue').textContent).toContain('looking for claude on PATH'),
  );
  expect((id('prm-settings-chat-path') as HTMLInputElement).value).toBe('tools/evil.exe');
  fireEvent.click(id('prm-settings-chat-path-clear'));
  await waitFor(() => expect(screen.queryByTestId('prm-settings-chat-path-issue')).toBeNull());
  expect(fake.callsOf('prm_chat_settings_put').at(-1)).toEqual({ claude_path: null });
  expect((id('prm-settings-chat-path') as HTMLInputElement).value).toBe('');
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

test('BL-41 the panel follows the open project by event, without polling', async () => {
  setup({ chat: { consent: true } });
  render(<ChatPanel />);
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
  render(<ChatPanel />);
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

test('R103 FR-PRM-26 the strip follows a context publish without a refocus', async () => {
  setup({ chat: { consent: true } });
  chatContext.reset();
  installChatSink();
  const w = window as unknown as { __pfMockHandlers: Record<string, (i: never) => unknown> };
  w.__pfMockHandlers.project_open = () => baseSnapshot();
  projectStore.reset();
  await act(() => projectStore.open(baseSnapshot().project_dir));
  const preview = w.__pfMockHandlers.prm_chat_preview!;
  let refs = 0;
  w.__pfMockHandlers.prm_chat_preview = (i: never) => ({
    ...(preview(i) as object),
    summary: `${refs} images · prompt · fake model`,
  });
  await act(async () => {
    chatContext.publish('gen', { refs: 0 });
    await chatContext.flush();
  });
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-preview').textContent).toContain('0 images'));
  // Same host, new input (8 references turned on): no host change, no focus event.
  refs = 8;
  await act(async () => {
    chatContext.publish('gen', { refs: 8 });
    await chatContext.flush();
  });
  await waitFor(() => expect(id('prm-chat-preview').textContent).toContain('8 images'));
  chatContext.reset();
});

test("BL-42 FR-PRM-23 a send first flushes the visible tab's pending edit", async () => {
  setup({ chat: { consent: true } });
  chatContext.reset();
  installChatSink();
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_open = () => baseSnapshot();
  projectStore.reset();
  await act(() => projectStore.open(baseSnapshot().project_dir));
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'old' });
  render(<ChatPanel />);
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

test('FR-PRM-46 #61 New chat starts an empty thread with a divider; disabled while a reply streams', async () => {
  setup({ chat: { consent: true, case: 'slow(5)' } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  // Nothing to replace yet.
  expect((id('prm-chat-new') as HTMLButtonElement).disabled).toBe(true);
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(id('prm-chat-stop')).toBeTruthy());
  expect((id('prm-chat-new') as HTMLButtonElement).disabled).toBe(true);
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1), {
    timeout: 5000,
  });
  expect((id('prm-chat-new') as HTMLButtonElement).disabled).toBe(false);
  expect(screen.queryByTestId('prm-chat-divider')).toBeNull();
  fireEvent.click(id('prm-chat-new'));
  await waitFor(() => expect(id('prm-chat-divider').textContent).toBe('New chat'));
  expect(fake.callsOf('prm_chat_thread_new')).toEqual([{ project_id: PID }]);
  expect(screen.queryAllByTestId('prm-chat-proposal')).toHaveLength(0);
  expect((id('prm-chat-new') as HTMLButtonElement).disabled).toBe(true);
  // The first turn of the new chat carries no "New session" note.
  await api.chat.fakeSetCase('success');
  await type('again');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  expect(screen.queryByText('New session')).toBeNull();
});

test('FR-PRM-47 #61 a fresh thread after an accept reloads the panel with its divider', async () => {
  setup({ chat: { consent: true } });
  await api.chat.contextPublish(PID, 'rgn', {});
  render(<ChatPanel />);
  await waitFor(() => expect(id('prm-chat-host').textContent).toBe('Region edit'));
  await type('ideas');
  fireEvent.click(id('prm-chat-send'));
  await waitFor(() => expect(screen.getAllByTestId('prm-chat-proposal')).toHaveLength(1));
  const startAfter = (window as unknown as { __pfPrmStartAfter: (p: string, a: object) => void })
    .__pfPrmStartAfter;
  act(() => startAfter(PID, { reason: 'accepted', revision_id: 'r', seq: 12 }));
  await waitFor(() =>
    expect(id('prm-chat-divider').textContent).toBe('New chat after accepting r12'),
  );
  expect(screen.queryAllByTestId('prm-chat-proposal')).toHaveLength(0);
});

test('FR-PRM-21 walk-61 L1 the dock never squeezes the canvas: it narrows with the window, the remembered width stays', async () => {
  setup({ chat: { consent: true, dockOpen: true } });
  await api.chat.settingsPut({ dock: { width: 600 } });
  const before = fake.callsOf('prm_chat_settings_put').length;
  render(<ChatDock />);
  await waitFor(() => expect(id('prm-chat-input')).toBeTruthy());
  const dock = id('prm-dock') as HTMLElement;
  expect(dock.style.width).toBe('600px');
  // The window shrinks: the canvas keeps its 480 px, so the dock gets 450.
  act(() => setDockRoom(450));
  expect(dock.style.width).toBe('450px');
  expect(id('prm-dock-resize').getAttribute('aria-valuemax')).toBe('450');
  // Even less room: the dock still opens at its minimum.
  act(() => setDockRoom(100));
  expect(dock.style.width).toBe('320px');
  // The window grows again: back to the user's width; nothing was saved meanwhile.
  act(() => setDockRoom(900));
  expect(dock.style.width).toBe('600px');
  expect(fake.callsOf('prm_chat_settings_put')).toHaveLength(before);
  // A drag stops at what the window allows.
  act(() => setDockRoom(500));
  const handle = id('prm-dock-resize');
  fireEvent.pointerDown(handle, { clientX: 1000, pointerId: 1 });
  fireEvent.pointerMove(handle, { clientX: 700, pointerId: 1 });
  expect(dock.style.width).toBe('500px');
  fireEvent.pointerUp(handle, { pointerId: 1 });
  expect(fake.callsOf('prm_chat_settings_put').at(-1)).toEqual({ dock: { width: 500 } });
  act(() => setDockRoom(null));
});
