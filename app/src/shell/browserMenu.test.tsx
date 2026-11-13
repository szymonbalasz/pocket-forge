import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'vitest';

import { ContextMenu } from '../shared/ContextMenu';
import { installBrowserMenuBlock } from './browserMenu';

let uninstall: () => void = () => undefined;
beforeEach(() => {
  uninstall = installBrowserMenuBlock();
});
afterEach(() => {
  uninstall();
  cleanup();
});

/** `fireEvent` returns false once something prevented the default (no browser menu). */
const browserMenuShows = (el: Element) => fireEvent.contextMenu(el);

test('#63 owner 2026-11-13: a right-click on a plain area shows no browser menu', () => {
  render(
    <div data-testid="plain">
      <p data-testid="text">Some text</p>
    </div>,
  );
  expect(browserMenuShows(screen.getByTestId('plain'))).toBe(false);
  expect(browserMenuShows(screen.getByTestId('text'))).toBe(false);
  expect(browserMenuShows(document.body)).toBe(false);
});

test('#63 owner 2026-11-13: text inputs, textareas and content-editable keep the native menu; a checkbox does not', () => {
  render(
    <div>
      <input data-testid="input" />
      <input data-testid="search" type="search" />
      <input data-testid="checkbox" type="checkbox" />
      <textarea data-testid="textarea" />
      <div data-testid="editable" contentEditable suppressContentEditableWarning>
        <span data-testid="editable-child">x</span>
      </div>
    </div>,
  );
  for (const id of ['input', 'search', 'textarea'])
    expect(browserMenuShows(screen.getByTestId(id))).toBe(true);
  // Integrator ruling: only text-entry fields; a checkbox has nothing to copy or paste.
  expect(browserMenuShows(screen.getByTestId('checkbox'))).toBe(false);
  expect(browserMenuShows(screen.getByTestId('editable-child'))).toBe(true);
});

test('#63 owner 2026-11-13: a Radix menu still opens (its trigger runs before the block)', async () => {
  let opened = false;
  render(
    <ContextMenu
      testId="t-menu"
      entries={[{ id: 'a', label: 'A', onSelect: () => undefined }]}
      onOpenChange={(o) => {
        opened ||= o;
      }}
    >
      <div data-testid="trigger">target</div>
    </ContextMenu>,
  );
  expect(browserMenuShows(screen.getByTestId('trigger'))).toBe(false);
  expect(opened).toBe(true);
  expect(await screen.findByTestId('t-menu-a')).toBeTruthy();
});

test('#63 the block is removed with its uninstall', () => {
  uninstall();
  render(<div data-testid="plain" />);
  expect(browserMenuShows(screen.getByTestId('plain'))).toBe(true);
});
