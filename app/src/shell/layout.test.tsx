import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';
import { useState } from 'react';

import { setDockRoom, useDockRoom } from '../shared/dockRoom';
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

// ── walk-61 L1: the dock never squeezes the canvas; the status bar never wraps ─────────────────

/** Stubs an element's layout width (jsdom has no layout). */
function widthOf(el: HTMLElement, w: () => number) {
  el.getBoundingClientRect = () => ({ width: w() }) as DOMRect;
}

function Room() {
  const room = useDockRoom();
  return <p data-testid="room">{room === null ? 'none' : room}</p>;
}

const panes: Feature = {
  id: 'p',
  prefix: 'p',
  slots: {
    left: () => <p>left</p>,
    toolbar: () => <p>tools</p>,
    dock: Room,
    right: Busy,
  },
};

test('FR-PRM-21 walk-61 L1: the dock gets what leaves the canvas 480 px, again on a window resize', () => {
  setDockRoom(null);
  render(<Layout features={[panes]} />);
  // Unmeasured (no layout): no limit.
  expect(screen.getByTestId('room').textContent).toBe('none');
  let win = 1400;
  let right = 340;
  widthOf(screen.getByTestId('app-shell'), () => win);
  widthOf(screen.getByTestId('app-slot-rail'), () => 56);
  widthOf(screen.getByTestId('app-slot-left'), () => 280);
  widthOf(screen.getByTestId('app-slot-toolbar'), () => 40);
  widthOf(screen.getByTestId('app-slot-right'), () => right);
  act(() => window.dispatchEvent(new Event('resize')));
  // 1400 − 56 − 280 − 40 − 340 − 480
  expect(screen.getByTestId('room').textContent).toBe('204');
  win = 1920;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(screen.getByTestId('room').textContent).toBe('724');
  // The right panel collapsed (Home): its width goes to the dock.
  right = 0;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(screen.getByTestId('room').textContent).toBe('1064');
  setDockRoom(null);
});

test('walk-61 L1: a narrow status bar hides segments by drop rank instead of wrapping', () => {
  const seg = (id: string, priority: number, drop?: number, grow?: boolean) => ({
    id,
    priority,
    ...(drop === undefined ? {} : { drop }),
    ...(grow ? { grow } : {}),
    Component: () => <span>{id}</span>,
  });
  const status: Feature = {
    id: 's',
    prefix: 's',
    slots: {},
    status: [
      seg('view', 10, 3),
      seg('save', 50, undefined, true),
      seg('job', 60, 2),
      seg('chip', 90),
      seg('chat', 91, 1),
    ],
  };
  const natural: Record<string, number> = { view: 300, save: 80, job: 110, chip: 150, chat: 50 };
  render(<Layout features={[status]} />);
  const bar = screen.getByTestId('app-status-bar');
  const segs = () => [...bar.querySelectorAll<HTMLElement>('[data-segment]')];
  let width = 1000;
  Object.defineProperty(bar, 'clientWidth', { get: () => width });
  Object.defineProperty(bar, 'scrollWidth', {
    get: () =>
      segs()
        .filter((s) => s.dataset.dropped === undefined)
        .reduce((sum, s) => sum + natural[s.dataset.segment!.split(':')[1]!]!, 0),
  });
  const shown = () =>
    segs()
      .filter((s) => s.dataset.dropped === undefined)
      .map((s) => s.dataset.segment!.split(':')[1]);
  act(() => window.dispatchEvent(new Event('resize')));
  expect(shown()).toEqual(['view', 'save', 'job', 'chip', 'chat']);
  // 690 needed: the chat toggle goes first.
  width = 650;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(shown()).toEqual(['view', 'save', 'job', 'chip']);
  width = 540;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(shown()).toEqual(['view', 'save', 'chip']);
  // Segments without a rank never hide.
  width = 100;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(shown()).toEqual(['save', 'chip']);
  // Room again: everything comes back.
  width = 1000;
  act(() => window.dispatchEvent(new Event('resize')));
  expect(shown()).toEqual(['view', 'save', 'job', 'chip', 'chat']);
});
