import { describe, expect, test, vi } from 'vitest';

import { chord, dispatchShortcut, isControlKey } from './shortcuts';

const key = (init: KeyboardEventInit, target?: EventTarget) => {
  const e = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  if (target) Object.defineProperty(e, 'target', { value: target });
  return e;
};

describe('shortcuts', () => {
  test('chords use physical brackets and Ctrl for Meta', () => {
    expect(chord(key({ key: 'b' }))).toBe('B');
    expect(chord(key({ key: '{', code: 'BracketLeft', shiftKey: true }))).toBe('Shift+[');
    expect(chord(key({ key: 'z', ctrlKey: true, shiftKey: true }))).toBe('Ctrl+Shift+Z');
    expect(chord(key({ key: '0', metaKey: true }))).toBe('Ctrl+0');
  });

  test('FR-EDT-30 single-key and bracket shortcuts are ignored in text fields', () => {
    const b = vi.fn();
    const br = vi.fn();
    const undo = vi.fn();
    const fit = vi.fn();
    const map = { B: b, ']': br, 'Ctrl+Z': undo, 'Ctrl+0': fit };
    const input = document.createElement('input');
    const area = document.createElement('textarea');
    const editable = document.createElement('div');
    Object.defineProperty(editable, 'isContentEditable', { value: true });
    const range = Object.assign(document.createElement('input'), { type: 'range' });
    for (const t of [input, area, editable]) {
      expect(dispatchShortcut(key({ key: 'b' }, t), map)).toBe(false);
      expect(dispatchShortcut(key({ key: ']', code: 'BracketRight' }, t), map)).toBe(false);
      expect(dispatchShortcut(key({ key: 'z', ctrlKey: true }, t), map)).toBe(false);
    }
    expect(b).not.toHaveBeenCalled();
    expect(br).not.toHaveBeenCalled();
    expect(undo).not.toHaveBeenCalled();
    // A Ctrl preset still works in a field; single keys work on non-text controls and the body.
    expect(dispatchShortcut(key({ key: '0', ctrlKey: true }, input), map)).toBe(true);
    expect(dispatchShortcut(key({ key: 'b' }, range), map)).toBe(true);
    expect(dispatchShortcut(key({ key: 'b' }, document.body), map)).toBe(true);
    expect(fit).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(2);
  });

  test('FR-EDT-25 FR-EDT-30 Enter and Space go to a focused button or control first (#48)', () => {
    const enter = vi.fn();
    const space = vi.fn();
    const b = vi.fn();
    const map = { Enter: enter, Space: space, B: b };
    const button = document.createElement('button');
    const link = Object.assign(document.createElement('a'), { href: '#x' });
    const check = Object.assign(document.createElement('input'), { type: 'checkbox' });
    const roleButton = document.createElement('div');
    roleButton.setAttribute('role', 'button');
    const icon = document.createElement('span');
    button.append(icon);
    for (const t of [button, link, check, roleButton, icon]) {
      expect(isControlKey(key({ key: 'Enter' }, t))).toBe(true);
      expect(isControlKey(key({ key: ' ' }, t))).toBe(true);
      expect(dispatchShortcut(key({ key: 'Enter' }, t), map)).toBe(false);
      expect(dispatchShortcut(key({ key: ' ' }, t), map)).toBe(false);
    }
    expect(enter).not.toHaveBeenCalled();
    expect(space).not.toHaveBeenCalled();
    // Other single keys still reach the shortcut on a focused button.
    expect(dispatchShortcut(key({ key: 'b' }, button), map)).toBe(true);
    // On the canvas (a plain focusable div) or the body, Enter and Space are shortcuts.
    const canvas = Object.assign(document.createElement('div'), { tabIndex: -1 });
    expect(isControlKey(key({ key: ' ' }, canvas))).toBe(false);
    expect(dispatchShortcut(key({ key: 'Enter' }, canvas), map)).toBe(true);
    expect(dispatchShortcut(key({ key: ' ' }, document.body), map)).toBe(true);
    expect(enter).toHaveBeenCalledTimes(1);
    expect(space).toHaveBeenCalledTimes(1);
  });
});
