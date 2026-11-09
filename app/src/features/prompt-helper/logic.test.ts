import { describe, expect, test } from 'vitest';

import type { PromptHelper } from '../../api';
import { fixWorthShowing, replySegments } from './chatLogic';
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
