// PRM slice 5 on the mocked core: Replace reaches only the turn host's prompt block
// (FR-PRM-29), and the chat API + mock the window (slice 6) builds on: consent, send with
// streamed deltas, the thread, Stop and Replace.
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';

import { api, onEvent, type Events } from '../../api';
import type { FakeTauri } from '../../api/mocks/tauri';
import { baseSnapshot } from '../../api/mocks/tauri';
import { projectStore } from '../../store/project';
import { mockPrm, TestHost } from './testUtils';

let fake: FakeTauri;
const PID = baseSnapshot().project_id;

afterEach(() => {
  cleanup();
  fake.uninstall();
});

async function openProject(): Promise<void> {
  const w = window as unknown as { __pfMockHandlers: Record<string, () => unknown> };
  w.__pfMockHandlers.project_open = () => baseSnapshot();
  projectStore.reset();
  await act(() => projectStore.open(baseSnapshot().project_dir));
}

const field = () => screen.getByTestId('prm-block-prompt') as HTMLTextAreaElement;
const insert = (host: 'rgn' | 'gen', text: string, project_id = PID) =>
  act(() => fake.emit('prm:insert', { project_id, host, proposal_id: 'p1', text }));

test('FR-PRM-29 Replace lands only in the turn host block of the open project', async () => {
  fake = mockPrm();
  await openProject();
  render(<TestHost host="rgn" prompt="old" />);
  await waitFor(() => expect(field().value).toBe('old'));
  insert('gen', 'for the other tab');
  insert('rgn', 'another project', '01JOTHERPROJECT00000000000');
  expect(field().value).toBe('old');
  insert('rgn', 'a kitchen with a stone floor');
  await waitFor(() => expect(field().value).toBe('a kitchen with a stone floor'));
});

test('FR-PRM-29 a read-only block ignores Replace', async () => {
  fake = mockPrm();
  await openProject();
  render(<TestHost host="gen" prompt="kept" readOnly />);
  insert('gen', 'changed');
  expect(field().value).toBe('kept');
});

function next<E extends keyof Events>(name: E): Promise<Events[E]> {
  return new Promise((resolve) => {
    let off: (() => void) | null = null;
    void onEvent(name, (p) => {
      off?.();
      resolve(p);
    }).then((u) => (off = u));
  });
}

test('FR-PRM-20 FR-PRM-27 the chat API on the mock: consent, send, stream, thread, Replace', async () => {
  fake = mockPrm();
  expect((await api.chat.status()).backend.state).toBe('not_checked');
  await expect(api.chat.send(PID, 'hi')).rejects.toMatchObject({ code: 'consent_required' });
  const st = await api.chat.consent(true);
  expect(st.backend).toMatchObject({ state: 'ok', notional: true });
  await expect(api.chat.send(PID, 'hi')).rejects.toMatchObject({ code: 'no_prompt_block' });

  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  const deltas: string[] = [];
  void onEvent('prm:chat-delta', (d) => deltas.push(d.text));
  const ended = next('prm:chat-turn');
  const out = await api.chat.send(PID, 'make this a kitchen');
  expect(out).toMatchObject({ host: 'rgn', fresh_session: true, saved: true });
  const end = await ended;
  expect(end).toMatchObject({ turn_id: out.turn_id, state: 'done' });
  expect(end.proposals).toHaveLength(1);
  expect(deltas.join('')).toContain('```prompt\nmake this a kitchen\n```');

  const view = await api.chat.thread(PID);
  expect(view.thread!.turns.map((t) => [t.role, t.host, t.state])).toEqual([
    ['user', 'rgn', 'done'],
    ['assistant', 'rgn', 'done'],
  ]);
  expect(view.fresh_session_next).toBe(false);

  // Replace while the Region edit block is hidden → no_prompt_block; attached → prm:insert.
  await api.chat.contextClear(PID, 'rgn');
  const proposal = end.proposals![0]!.id;
  await expect(api.chat.insert(PID, proposal)).rejects.toMatchObject({ code: 'no_prompt_block' });
  await api.chat.contextPublish(PID, 'rgn', { final_prompt: 'a kitchen' });
  const inserted = next('prm:insert');
  await api.chat.insert(PID, proposal);
  expect(await inserted).toMatchObject({ host: 'rgn', text: 'make this a kitchen' });
});

test('FR-PRM-27 Stop keeps the partial reply on the mock', async () => {
  fake = mockPrm({ chat: { consent: true, case: 'slow(4)' } });
  await api.chat.contextPublish(PID, 'gen', {});
  const firstDelta = next('prm:chat-delta');
  const ended = next('prm:chat-turn');
  const out = await api.chat.send(PID, 'a crypt');
  await firstDelta;
  await api.chat.cancel(out.turn_id);
  expect(await ended).toMatchObject({ state: 'canceled' });
  const reply = (await api.chat.thread(PID)).thread!.turns[1]!;
  expect(reply.state).toBe('canceled');
  expect(reply.text.length).toBeGreaterThan(0);
  expect((await api.chat.thread(PID)).fresh_session_next).toBe(true);
});
