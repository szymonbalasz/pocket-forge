// The prompt chat's live context seam (FR-PRM-23, FR-PRM-43, FR-GEN-36).
import { afterEach, beforeEach, expect, test, vi } from 'vitest';

import { CHAT_PUBLISH_MS, chatContext, type ChatContextSink } from './chatContext';

const sink = () => ({ publish: vi.fn(), clear: vi.fn() }) satisfies ChatContextSink;

beforeEach(() => {
  vi.useFakeTimers();
  chatContext.reset();
});
afterEach(() => vi.useRealTimers());

test('FR-PRM-23 publish is debounced 300 ms; the newest input wins', () => {
  const s = sink();
  chatContext.setSink(s);
  chatContext.publish('gen', { typed_prompt: 'a' });
  chatContext.publish('gen', { typed_prompt: 'ab' });
  vi.advanceTimersByTime(CHAT_PUBLISH_MS - 1);
  expect(s.publish).not.toHaveBeenCalled();
  expect(chatContext.current()).toBeNull();
  vi.advanceTimersByTime(1);
  expect(s.publish).toHaveBeenCalledTimes(1);
  expect(s.publish).toHaveBeenCalledWith('gen', { typed_prompt: 'ab' });
  expect(chatContext.current()).toEqual({ host: 'gen', input: { typed_prompt: 'ab' } });
});

test('FR-PRM-23 flush publishes the pending input right before a send', () => {
  const s = sink();
  chatContext.setSink(s);
  chatContext.publish('rgn', { x: 1 });
  void chatContext.flush();
  expect(s.publish).toHaveBeenCalledWith('rgn', { x: 1 });
  vi.advanceTimersByTime(CHAT_PUBLISH_MS);
  expect(s.publish).toHaveBeenCalledTimes(1);
  void chatContext.flush(); // nothing pending: no-op
  expect(s.publish).toHaveBeenCalledTimes(1);
});

test('BL-42 flush resolves once the sink has the input', async () => {
  let release: () => void = () => undefined;
  const s = {
    publish: vi.fn(() => new Promise<void>((r) => (release = r))),
    clear: vi.fn(),
  };
  chatContext.setSink(s);
  chatContext.publish('rgn', { x: 2 });
  let done = false;
  const p = chatContext.flush().then(() => (done = true));
  await Promise.resolve();
  expect(done).toBe(false);
  release();
  await p;
  expect(done).toBe(true);
});

test('FR-GEN-36 FR-PRM-43 a hidden host clears its own context only; the new host publishes', () => {
  const s = sink();
  chatContext.setSink(s);
  chatContext.publish('gen', { g: 1 });
  vi.advanceTimersByTime(CHAT_PUBLISH_MS);
  // The tab switch: the New image block unmounts, the Region edit block publishes.
  chatContext.clear('gen');
  expect(s.clear).toHaveBeenCalledWith('gen');
  expect(chatContext.current()).toBeNull();
  chatContext.publish('rgn', { r: 1 });
  chatContext.clear('gen'); // a late clear of the old host keeps the new one's pending input
  vi.advanceTimersByTime(CHAT_PUBLISH_MS);
  expect(chatContext.current()).toEqual({ host: 'rgn', input: { r: 1 } });
  // A pending publish of a host that is then hidden never goes out.
  chatContext.publish('gen', { g: 2 });
  chatContext.clear('gen');
  vi.advanceTimersByTime(CHAT_PUBLISH_MS);
  expect(s.publish).not.toHaveBeenCalledWith('gen', { g: 2 });
});

test('FR-PRM-23 a sink installed later (PRM slice 5) gets the current context at once', () => {
  chatContext.publish('gen', { g: 1 });
  vi.advanceTimersByTime(CHAT_PUBLISH_MS);
  const s = sink();
  chatContext.setSink(s);
  expect(s.publish).toHaveBeenCalledWith('gen', { g: 1 });
});
