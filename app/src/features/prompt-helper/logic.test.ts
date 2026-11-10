import { describe, expect, test } from 'vitest';

import type { ChatStatus, ChatThread, ChatTurn, PromptHelper } from '../../api';
import { billsApiKey, dividerText, fixWorthShowing, replySegments, usageLabel } from './chatLogic';
import { effectiveTicks, fieldError, moveWithin, rowHelpers, toggleTick } from './logic';

const h = (id: string, patch: Partial<PromptHelper> = {}): PromptHelper => ({
  id,
  label: id,
  text: `${id}.`,
  kinds: ['map'],
  default_on: false,
  builtin: false,
  hidden: false,
  modified: false,
  ...patch,
});

describe('prompt-helper logic', () => {
  const list = [
    h('a', { default_on: true }),
    h('b', { kinds: ['token'], default_on: true }),
    h('c', { hidden: true, default_on: true }),
    h('d'),
  ];

  test('FR-PRM-04 the row is the visible helpers of the kind, in order', () => {
    expect(rowHelpers(list, 'map').map((x) => x.id)).toEqual(['a', 'd']);
  });

  test('FR-PRM-05 null = the visible default_on helpers; a toggle writes the explicit list', () => {
    expect(effectiveTicks(null, list, 'map')).toEqual(['a']);
    expect(effectiveTicks([], list, 'map')).toEqual([]);
    expect(toggleTick(null, list, 'map', 'd')).toEqual(['a', 'd']);
    expect(toggleTick(null, list, 'map', 'a')).toEqual([]);
    expect(toggleTick(['x'], list, 'map', 'a')).toEqual(['x', 'a']);
  });

  test('FR-PRM-01 field errors', () => {
    expect(fieldError({ label: ' ', text: 'x', kinds: ['map'] })?.field).toBe('label');
    expect(fieldError({ label: 'é'.repeat(40), text: 'x', kinds: ['map'] })).toBeNull();
    expect(fieldError({ label: 'a', text: 'x'.repeat(501), kinds: ['map'] })?.field).toBe('text');
    expect(fieldError({ label: 'a', text: 'x', kinds: [] })?.field).toBe('kinds');
  });

  test('FR-PRM-04 move swaps with the nearest row of the same kind', () => {
    const rows = [
      { key: 'm1', kinds: ['map' as const] },
      { key: 't1', kinds: ['token' as const] },
      { key: 'm2', kinds: ['map' as const] },
    ];
    expect(moveWithin(rows, 'm2', 'map', -1).map((r) => r.key)).toEqual(['m2', 't1', 'm1']);
    expect(moveWithin(rows, 'm1', 'map', -1).map((r) => r.key)).toEqual(['m1', 't1', 'm2']);
    expect(moveWithin(rows, 'm2', 'map', 1).map((r) => r.key)).toEqual(['m1', 't1', 'm2']);
  });
});

describe('§7 L6 the chat transcript', () => {
  test('FR-PRM-28 a prompt block that became a card is not shown again in the reply', () => {
    const reply = 'Ideas:\n```prompt\na kitchen\n```\n```text\nnote\n```\n```prompt\nextra\n```';
    const shown = (n: number) => replySegments(reply, n).map((s) => s.text);
    expect(shown(0)).toEqual(['Ideas:', 'a kitchen', 'note', 'extra']);
    expect(shown(1)).toEqual(['Ideas:', 'note', 'extra']);
    expect(shown(2)).toEqual(['Ideas:', 'note']);
  });

  test('FR-PRM-37 a fix that only says "Retry." is left out next to the Retry button', () => {
    expect(fixWorthShowing('Retry.')).toBe(false);
    expect(fixWorthShowing(undefined)).toBe(false);
    expect(fixWorthShowing('Retry; details show the stderr tail.')).toBe(true);
  });
});

describe('FR-PRM-38 usage', () => {
  const turn = (usage?: ChatTurn['usage']): ChatTurn => ({
    id: 't',
    at: '2026-01-01T00:00:00Z',
    role: 'assistant',
    host: 'rgn',
    text: '',
    state: 'done',
    ...(usage ? { usage } : {}),
  });
  const u = (cost_usd: number, notional: boolean) => ({
    input_tokens: 1,
    output_tokens: 1,
    cost_usd,
    notional,
  });

  test('FR-PRM-38 the footer totals the thread: plan usage, billed, or the fake', () => {
    expect(usageLabel([turn()])).toBeNull();
    expect(usageLabel([turn(u(0.016, true)), turn(), turn(u(0.024, true))])).toBe(
      '≈ $0.04 plan usage',
    );
    expect(usageLabel([turn(u(0.03, true)), turn(u(0.01, false))])).toBe('≈ $0.04 billed');
    expect(usageLabel([turn(u(0.01, true))], true)).toBe(
      '≈ $0.01 notional (fake chat, nothing billed)',
    );
  });

  test('FR-PRM-38 only an api_key login warns', () => {
    const st = (backend: ChatStatus['backend']) => ({ backend }) as ChatStatus;
    const ok = (auth_method: string | null) =>
      st({ state: 'ok', version: '2.1.319', auth_method, plan: null, notional: false } as never);
    expect(billsApiKey(ok('api_key'))).toBe(true);
    expect(billsApiKey(ok('claude.ai'))).toBe(false);
    expect(billsApiKey(ok(null))).toBe(false);
    expect(billsApiKey(st({ state: 'not_checked' } as never))).toBe(false);
  });
});

test('FR-PRM-47 #61 the divider says why a thread started', () => {
  const t = (extra: object) =>
    ({
      schema_version: 1,
      id: 't',
      created_at: '',
      updated_at: '',
      backend: 'fake',
      turns: [],
      ...extra,
    }) as ChatThread;
  expect(dividerText(t({}))).toBeNull();
  expect(dividerText(t({ previous_thread_id: 'a' }))).toBe('New chat');
  expect(
    dividerText(
      t({
        previous_thread_id: 'a',
        started_after: { reason: 'accepted', revision_id: 'r', seq: 12 },
      }),
    ),
  ).toBe('New chat after accepting r12');
  expect(
    dividerText(
      t({ previous_thread_id: 'a', started_after: { reason: 'picked', revision_id: 'r', seq: 3 } }),
    ),
  ).toBe('New chat after picking r3');
});
