// The GEN and RGN api mocks follow pf-core after R97/R98 (the UI tests and e2e trust them):
// undo keeps `draft.generate`, generate candidate statuses follow the restored base, a pick is
// refused while a job runs, and a region edit candidate made on another base isn't accepted.
import { afterEach, beforeEach, expect, test } from 'vitest';

import type { FakeTauri } from '../../api/mocks/tauri';
import { mockGenCore, rgnControls } from './testUtils';

type Rec = Record<string, unknown>;
const w = window as unknown as {
  __pfMockHandlers: Record<string, (i: Rec) => unknown>;
  __pfMockSnapshot: Rec;
};
const call = async (cmd: string, input: Rec = {}) => (await w.__pfMockHandlers[cmd]!(input)) as Rec;
const err = (cmd: string, input: Rec) =>
  call(cmd, input).then(
    () => {
      throw new Error(`${cmd} should have failed`);
    },
    (e: { code: string; message: string; field?: string }) => e,
  );
const state = () =>
  (window as unknown as { __pfRgnMock: { state(): Rec } }).__pfRgnMock.state() as Rec & {
    jobs: Rec[];
    cands: (Rec & { id: string; job_id: string; status: string; trash_id?: unknown })[];
    layers: Rec[];
  };

let fake: FakeTauri;
beforeEach(() => {
  fake = mockGenCore({});
});
afterEach(() => fake.uninstall());

async function generate(prompt = 'flooded crypt', count = 2) {
  const draft = {
    model_id: 'fake-gen-wh',
    final_prompt: prompt,
    controls: {},
    size: { mode: 'ratio', aspect: '1:1', long_side: 512 },
  };
  const est = await call('gen_job_estimate', { draft });
  const { job_id } = await call('gen_job_submit', {
    draft,
    estimate_id: est.estimate_id,
    confirmations: est.needs,
  });
  rgnControls().finish(count);
  const cands = state().cands.filter((c) => c.job_id === job_id);
  expect(cands).toHaveLength(count);
  return { job_id: job_id as string, cands };
}

async function submitRegionEdit() {
  const draft = {
    model_id: 'fake-all-types',
    final_prompt: 'a kitchen',
    controls: {},
    seed: null,
    region: { inner: { x: 0, y: 0, w: 512, h: 512 }, margin: 64 },
    mask_id: '01JMASK000000000000000000',
  };
  const est = await call('rgn_job_estimate', { draft });
  const { job_id } = await call('rgn_job_submit', {
    draft,
    estimate_id: est.estimate_id,
    confirmations: est.needs,
  });
  return job_id as string;
}

const pick = (job_id: string, candidate_id: string) =>
  call('gen_candidate_pick', { job_id, candidate_id });
const statusOf = (id: string) => state().cands.find((c) => c.id === id)!.status;

test('FR-GEN-19 FR-PRJ-23 R98 mock: undo of a pick keeps draft.generate', async () => {
  const { job_id, cands } = await generate();
  const generate_ = { prompt: 'flooded crypt', size: { mode: 'ratio', aspect: '1:1' } };
  w.__pfMockSnapshot.draft = { ...(w.__pfMockSnapshot.draft as Rec), generate: generate_ };
  const first = await pick(job_id, cands[0]!.id);
  await pick(job_id, cands[1]!.id);
  await call('project_restore_revision', { rev_id: first.revision_id });
  expect((w.__pfMockSnapshot.draft as Rec).generate).toEqual(generate_);
  await call('project_restore_revision', { rev_id: 'r0' });
  expect((w.__pfMockSnapshot.draft as Rec).generate).toEqual(generate_);
});

test('FR-GEN-29 R97 mock: candidate statuses follow the restored base, a trashed one comes back', async () => {
  const { job_id, cands } = await generate();
  const [a, b] = [cands[0]!.id, cands[1]!.id];
  const first = await pick(job_id, a);
  await pick(job_id, b);
  expect([statusOf(a), statusOf(b)]).toEqual(['dismissed', 'accepted']);
  state().cands.find((c) => c.id === a)!.trash_id = '01JTRASH0001';
  await call('project_restore_revision', { rev_id: first.revision_id });
  expect([statusOf(a), statusOf(b)]).toEqual(['accepted', 'dismissed']);
  expect(state().cands.find((c) => c.id === a)!.trash_id).toBeNull();
  // Back to the original image: no generate candidate is accepted.
  await call('project_restore_revision', { rev_id: 'r0' });
  expect([statusOf(a), statusOf(b)]).toEqual(['dismissed', 'dismissed']);
});

test('FR-GEN-30 R97 mock: a pick is refused with job_active while a job runs', async () => {
  const { job_id, cands } = await generate();
  const running = await submitRegionEdit();
  const e = await err('gen_candidate_pick', { job_id, candidate_id: cands[0]!.id });
  expect(e.code).toBe('job_active');
  expect(state().layers[0]!.kind).toBe('original');
  rgnControls().finish(1);
  expect(state().jobs.find((j) => j.id === running)!.state).toBe('fetched');
  await pick(job_id, cands[0]!.id);
  expect(state().layers[0]!.kind).toBe('base');
});

test('FR-RGN-40 R97 mock: a region edit candidate made on another base image is not accepted', async () => {
  const { job_id, cands } = await generate();
  const edit = await submitRegionEdit();
  rgnControls().finish(1);
  const cand = state().cands.find((c) => c.job_id === edit)!;
  await pick(job_id, cands[0]!.id);
  const e = await err('rgn_candidate_accept', {
    job_id: edit,
    candidate_id: cand.id,
    blend: { feather_px: 8 },
  });
  expect(e.code).toBe('invalid_candidate');
  expect(e.message).toContain('different base image');
  expect(statusOf(cand.id)).toBe('new');
  // On its own base it is accepted.
  const again = await submitRegionEdit();
  rgnControls().finish(1);
  const ok = state().cands.find((c) => c.job_id === again)!;
  await call('rgn_candidate_accept', {
    job_id: again,
    candidate_id: ok.id,
    blend: { feather_px: 8 },
  });
  expect(statusOf(ok.id)).toBe('accepted');
});
