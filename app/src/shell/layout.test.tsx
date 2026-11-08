import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';
import { useState } from 'react';

import type { Feature } from '../shared/feature';
import { useSlotIdle } from '../shared/slot';
import { Layout } from './Layout';

afterEach(cleanup);

let setOpen: (open: boolean) => void = () => undefined;

function Idling() {
  const [open, set] = useState(false);
  setOpen = set;
  useSlotIdle(!open);
  return open ? <p>panel</p> : null;
}

function Disabled() {
  const [open] = useState(false);
  useSlotIdle(!open);
  return <p>disabled panel</p>;
}

function Busy() {
  return <p>always here</p>;
}

const feature = (id: string, C: Feature['slots']['right']): Feature => ({
  id,
  prefix: id,
  slots: { right: C },
});

test('ui-guidelines BL-15: the right panel collapses while every component in it is idle', () => {
  render(<Layout features={[feature('a', Idling), feature('b', Disabled)]} />);
  const right = screen.getByTestId('app-slot-right');
  expect(right.hidden).toBe(true);
  act(() => setOpen(true));
  expect(right.hidden).toBe(false);
  act(() => setOpen(false));
  expect(right.hidden).toBe(true);
});

test('ui-guidelines BL-15: a component that never reports keeps the right panel open', () => {
  render(<Layout features={[feature('a', Idling), feature('c', Busy)]} />);
  expect(screen.getByTestId('app-slot-right').hidden).toBe(false);
});
