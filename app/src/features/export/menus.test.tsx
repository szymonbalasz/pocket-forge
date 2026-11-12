// #63 context menus (EXP §7, FR-EXP-30..33, FR-PRJ-59) against the mocked core: the shared menu's
// items and disabled reasons, the toast (progress, Replace, Cancel, done), the save dialog through
// the api seam, and a menu export counting as the project's running export.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Tooltip } from 'radix-ui';
import { afterEach, beforeEach, expect, test } from 'vitest';

import type { ExportPlan } from '../../api';
import { prjMock, samplePrjSeed, type PrjMockState } from '../../api/mocks/prj';
import { installTestCore } from '../../api/mocks/testCore';
import { projectStore } from '../../store/project';
import { CanvasMenu, FileMenu, PackImageMenu, revealEntry } from './Menus';
import { MenuToastView } from './Menus';
import { ExportPanel } from './ExportPanel';
import { menuExport } from './menu';
import { exportStore } from './store';
import { startState } from './view';
import { ContextMenu } from '../../shared/ContextMenu';

const core = installTestCore();
(globalThis as unknown as Record<string, unknown>).ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const LIB = 'C:/pf-mock/library';
const CRYPT = `${LIB}/Maps/Crypt`;
const HARBOUR = `${LIB}/Maps/Harbour`;
let mock: PrjMockState;

const plan = (webp: string | null = null): ExportPlan => ({
  width: 512,
  height: 640,
  scale: 1,
  formats: [
    { format: 'png', available: true, reason: null },
    { format: 'jpeg', available: true, reason: null },
    { format: 'webp', available: webp === null, reason: webp },
  ],
  path: 'exports/Crypt.png',
  exists: null,
  est_peak_mib: 2,
  warnings: [],
});

const SETTINGS = {
  grid: { mode: 'px', source_px_per_square: 64 },
  target_px_per_square: 32,
  format: 'png',
};

/** The settings as the panel's form sends them. */
const SENT = { ...SETTINGS, grid: { ...SETTINGS.grid, cols: null, rows: null } };

type Handler = (input: Record<string, unknown>) => unknown;

async function setup(
  extra: Record<string, Handler> = {},
  path = CRYPT,
  webpReason: string | null = null,
) {
  const seed = samplePrjSeed(LIB);
  // Harbour opens read-only (FR-PRJ-41).
  seed.projects = seed.projects.map((p) =>
    p.name === 'Harbour' ? { ...p, read_only: 'original_changed' } : p,
  );
  const m = prjMock(seed, { emit: core.emit });
  mock = m.state;
  let n = 0;
  core.useHandlers({
    ...m.handlers,
    export_get_settings: () => ({
      ...SETTINGS,
      source_px_per_square: 64,
      target_px_per_square: 32,
    }),
    export_plan: () => plan(webpReason),
    project_prefetch_original: () => ({ ready: true }),
    export_start: () => ({ export_id: `01JEXP${String(++n).padStart(20, '0')}` }),
    export_cancel: (i) => {
      setTimeout(() => core.emit('export:canceled', { export_id: i.export_id }), 5);
      return {};
    },
    ...extra,
  });
  await projectStore.open(path);
  await exportStore.listen();
  await exportStore.bind(projectStore.get().project!.project_id);
  await waitFor(() => expect(exportStore.get().plan).not.toBeNull());
}

function openMenu(testId: string) {
  fireEvent.contextMenu(screen.getByTestId(testId));
}

async function exportAs(menu: string, format: 'png' | 'jpeg' | 'webp') {
  fireEvent.click(await screen.findByTestId(`${menu}-export`));
  fireEvent.click(await screen.findByTestId(`${menu}-export-${format}`));
}

const done = (id: string, path: string) =>
  act(async () =>
    core.emit('export:done', {
      export_id: id,
      path,
      width: 512,
      height: 640,
      bytes: 12_300_000,
      format: 'jpeg',
      fallback: null,
      record_saved: true,
    }),
  );

beforeEach(() => {
  projectStore.reset();
  exportStore.reset();
  menuExport.reset();
});
afterEach(cleanup);

test('FR-EXP-30 Export as on the canvas starts with format_override and the panel’s settings, and the toast ends with the path', async () => {
  await setup();
  render(
    <>
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </>,
  );
  openMenu('stage');
  expect(await screen.findByTestId('edt-canvas-menu')).toBeTruthy();
  expect(screen.getByTestId('edt-canvas-menu-save-as')).toBeTruthy();
  expect(screen.getByTestId('edt-canvas-menu-reveal')).toBeTruthy();
  await exportAs('edt-canvas-menu', 'jpeg');
  await waitFor(() => expect(core.names()).toContain('export_start'));
  const input = core.calls.find((c) => c.cmd === 'export_start')!.input;
  expect(input).toEqual({ settings: SENT, replace: false, format_override: 'jpeg' });
  // The panel's settings are not sent back (no export_set_settings).
  expect(core.names()).not.toContain('export_set_settings');
  const toast = await screen.findByTestId('exp-toast');
  expect(toast.dataset.source).toBe('menu');
  await waitFor(() => expect(toast.dataset.phase).toBe('running'));
  expect(screen.getByTestId('exp-toast-progress')).toBeTruthy();
  await done('01JEXP00000000000000000001', `${CRYPT}/exports/Crypt_32px.jpg`);
  await waitFor(() =>
    expect(screen.getByTestId('exp-toast-path').textContent).toBe(
      `${CRYPT}/exports/Crypt_32px.jpg`,
    ),
  );
  expect(screen.getByTestId('exp-toast-copy')).toBeTruthy();
});

test('FR-EXP-30 an existing file asks Replace in the toast and writes nothing until Replace', async () => {
  let calls = 0;
  await setup({
    export_start: (i) => {
      calls++;
      if (i.replace !== true)
        return Promise.reject({
          code: 'exists',
          message: 'exports/Crypt_32px.webp exists',
          detail: {
            path: 'exports/Crypt_32px.webp',
            bytes: 12_300_000,
            mtime: new Date(Date.now() - 2 * 86_400_000).toISOString(),
          },
        });
      return { export_id: '01JEXP00000000000000000009' };
    },
  });
  render(
    <>
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </>,
  );
  openMenu('stage');
  await exportAs('edt-canvas-menu', 'webp');
  const q = await screen.findByTestId('exp-toast-question');
  expect(q.textContent).toBe('Replace Crypt_32px.webp (11.7 MB, 2 days ago)?');
  expect(calls).toBe(1);
  fireEvent.click(screen.getByTestId('exp-toast-replace'));
  await waitFor(() => expect(calls).toBe(2));
  expect(core.calls.filter((c) => c.cmd === 'export_start')[1]!.input.replace).toBe(true);
  // Cancel on a Replace question just drops it.
  await act(async () => menuExport.reset());
});

test('FR-EXP-30 Export as is disabled with its reason: no image, read-only, a format too large', async () => {
  await setup({}, CRYPT, 'WebP max 16383 px');
  const { unmount } = render(
    <CanvasMenu>
      <div data-testid="stage" />
    </CanvasMenu>,
  );
  openMenu('stage');
  fireEvent.click(await screen.findByTestId('edt-canvas-menu-export'));
  const webp = await screen.findByTestId('edt-canvas-menu-export-webp');
  expect(webp.dataset.reason).toBe('WebP max 16383 px');
  expect(webp.hasAttribute('data-disabled')).toBe(true);
  expect(screen.getByTestId('edt-canvas-menu-export-png').hasAttribute('data-disabled')).toBe(
    false,
  );
  unmount();
  cleanup();

  // Harbour is read-only in the sample library.
  projectStore.reset();
  exportStore.reset();
  await setup({}, HARBOUR);
  render(
    <CanvasMenu>
      <div data-testid="stage" />
    </CanvasMenu>,
  );
  openMenu('stage');
  const sub = await screen.findByTestId('edt-canvas-menu-export');
  expect(sub.dataset.reason).toBe('The project is read-only');
  // Save as writes nothing to the project: still offered (FR-EXP-31).
  expect(screen.getByTestId('edt-canvas-menu-save-as').hasAttribute('data-disabled')).toBe(false);
});

test('FR-EXP-31 Save as opens the save dialog at export_save_default and starts with dest; Cancel does nothing', async () => {
  let picked: string | null = null;
  await setup({ 'plugin:dialog|save': () => picked });
  render(
    <>
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </>,
  );
  openMenu('stage');
  fireEvent.click(await screen.findByTestId('edt-canvas-menu-save-as'));
  await waitFor(() => expect(core.names()).toContain('plugin:dialog|save'));
  const opts = core.calls.find((c) => c.cmd === 'plugin:dialog|save')!.input.options as Record<
    string,
    unknown
  >;
  expect(opts.defaultPath).toBe(`${CRYPT}/exports/Crypt.png`);
  expect(opts.filters).toEqual([
    { name: 'PNG', extensions: ['png'] },
    { name: 'JPEG', extensions: ['jpg', 'jpeg'] },
    { name: 'WebP', extensions: ['webp'] },
  ]);
  await new Promise((r) => setTimeout(r, 20));
  expect(core.names()).not.toContain('export_start');

  picked = 'D:/out/vault.webp';
  openMenu('stage');
  fireEvent.click(await screen.findByTestId('edt-canvas-menu-save-as'));
  await waitFor(() => expect(core.names()).toContain('export_start'));
  expect(core.calls.find((c) => c.cmd === 'export_start')!.input).toEqual({
    settings: SENT,
    replace: true,
    dest: 'D:/out/vault.webp',
  });
});

test('FR-EXP-32 an image file’s menu exports into exports/ and saves as; other files only reveal', async () => {
  await setup({ 'plugin:dialog|save': () => 'D:/out/statue.jpg' });
  render(
    <>
      <FileMenu testId="prj-panel-file-menu" path={`${CRYPT}/references/statue.png`} image>
        <div data-testid="img" />
      </FileMenu>
      <FileMenu testId="prj-panel-file-menu" path={`${CRYPT}/references/notes.txt`} image={false}>
        <div data-testid="txt" />
      </FileMenu>
      <MenuToastView />
    </>,
  );
  openMenu('txt');
  await screen.findByTestId('prj-panel-file-menu');
  expect(screen.queryByTestId('prj-panel-file-menu-save-as')).toBeNull();
  expect(screen.queryByTestId('prj-panel-file-menu-export')).toBeNull();
  fireEvent.click(screen.getByTestId('prj-panel-file-menu-reveal'));
  await waitFor(() => expect(mock.reveals).toEqual([`/select,${CRYPT}/references/notes.txt`]));

  openMenu('img');
  await exportAs('prj-panel-file-menu', 'webp');
  await waitFor(() => expect(mock.fileExports).toHaveLength(1));
  expect(mock.fileExports[0]).toEqual({
    source: `${CRYPT}/references/statue.png`,
    dest: 'exports',
    format: 'webp',
  });
  await waitFor(() =>
    expect(screen.getByTestId('exp-toast-path').textContent).toBe(
      `${CRYPT}/exports/Crypt_statue.webp`,
    ),
  );
  openMenu('img');
  fireEvent.click(await screen.findByTestId('prj-panel-file-menu-save-as'));
  await waitFor(() => expect(mock.fileExports).toHaveLength(2));
  expect(mock.fileExports[1]).toEqual({
    source: `${CRYPT}/references/statue.png`,
    dest: 'D:/out/statue.jpg',
  });
});

test('FR-EXP-32 a pack image has Save as and Open in Explorer, no Export as', async () => {
  await setup();
  render(
    <PackImageMenu path={`${LIB}/References/Dragons/red.png`}>
      <div data-testid="pack-img" />
    </PackImageMenu>,
  );
  openMenu('pack-img');
  await screen.findByTestId('ref-pack-image-menu');
  expect(screen.getByTestId('ref-pack-image-menu-save-as')).toBeTruthy();
  expect(screen.queryByTestId('ref-pack-image-menu-export')).toBeNull();
  fireEvent.click(screen.getByTestId('ref-pack-image-menu-reveal'));
  await waitFor(() => expect(mock.reveals).toEqual([`/select,${LIB}/References/Dragons/red.png`]));
});

test('FR-EXP-33 a running menu export makes Start busy and a close ask first; the answer cancels it', async () => {
  await setup();
  render(
    <>
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </>,
  );
  openMenu('stage');
  await exportAs('edt-canvas-menu', 'png');
  await waitFor(() => expect(exportStore.get().menuBusy).toBe(true));
  expect(
    startState({
      hasProject: true,
      readOnly: null,
      original: { kind: 'ready' },
      plan: plan(),
      format: 'png',
      running: false,
      blocked: false,
      menuBusy: true,
    }),
  ).toMatchObject({ enabled: false, label: 'Busy' });
  // The close is held behind the export question.
  const closing = projectStore.close();
  await waitFor(() => expect(exportStore.get().closeAsk).toBe(true));
  expect(projectStore.get().project).not.toBeNull();
  await act(async () => exportStore.confirmClose());
  await closing;
  expect(core.names()).toContain('export_cancel');
  await waitFor(() => expect(projectStore.get().project).toBeNull());
});

test('FR-EXP-33 the toast’s Cancel cancels the menu export and the toast goes', async () => {
  await setup();
  render(
    <>
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </>,
  );
  openMenu('stage');
  await exportAs('edt-canvas-menu', 'png');
  const cancel = await screen.findByTestId('exp-toast-cancel');
  await waitFor(() => expect((cancel as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(cancel);
  await waitFor(() => expect(core.names()).toContain('export_cancel'));
  await waitFor(() => expect(screen.queryByTestId('exp-toast')).toBeNull());
  expect(exportStore.get().menuBusy).toBe(false);
});

test('FR-PRJ-59 Open in Explorer entries reveal the folder; a refusal shows in the toast', async () => {
  await setup({
    reveal_in_explorer: (i) =>
      String(i.path).includes('Gone')
        ? Promise.reject({ code: 'not_found', message: 'not found' })
        : (mock.reveals.push(String(i.path)), {}),
  });
  render(
    <>
      <ContextMenu
        testId="prj-home-card-menu"
        itemPrefix="prj-home-card"
        entries={[revealEntry(CRYPT)]}
      >
        <div data-testid="card" />
      </ContextMenu>
      <ContextMenu testId="x-menu" entries={[revealEntry(`${LIB}/Maps/Gone`)]}>
        <div data-testid="gone" />
      </ContextMenu>
      <MenuToastView />
    </>,
  );
  openMenu('card');
  fireEvent.click(await screen.findByTestId('prj-home-card-reveal'));
  await waitFor(() => expect(mock.reveals).toEqual([CRYPT]));
  openMenu('gone');
  fireEvent.click(await screen.findByTestId('x-menu-reveal'));
  expect((await screen.findByTestId('exp-toast-error')).textContent).toBe(
    'The file is gone (moved or deleted?)',
  );
});

test('FR-EXP-33 walk-63 H1 closing, renaming or deleting while a menu export runs shows the question in the panel', async () => {
  await setup();
  render(
    <Tooltip.Provider>
      <ExportPanel />
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
      <MenuToastView />
    </Tooltip.Provider>,
  );
  openMenu('stage');
  await exportAs('edt-canvas-menu', 'png');
  await waitFor(() => expect(exportStore.get().menuBusy).toBe(true));
  // The panel shows no run of its own; Start reads Busy.
  expect(screen.queryByTestId('exp-panel-progress')).toBeNull();

  const renaming = projectStore.rename('Vault');
  expect((await screen.findByTestId('exp-panel-close-question')).textContent).toBe(
    'Cancel the export and rename?',
  );
  fireEvent.click(screen.getByTestId('exp-panel-close-keep'));
  await renaming;
  await waitFor(() => expect(screen.queryByTestId('exp-panel-close-ask')).toBeNull());
  expect(projectStore.get().project?.name).toBe('Crypt');

  const closing = projectStore.close();
  expect((await screen.findByTestId('exp-panel-close-question')).textContent).toBe(
    'Cancel the export and close?',
  );
  expect(screen.getByTestId('exp-panel-close-confirm').textContent).toBe('Cancel export and close');
  fireEvent.click(screen.getByTestId('exp-panel-close-confirm'));
  await closing;
  expect(core.names()).toContain('export_cancel');
  await waitFor(() => expect(projectStore.get().project).toBeNull());
});

test('FR-EXP-33 walk-63 H1 the question goes, and the close goes on, when the menu export ends by itself', async () => {
  await setup();
  render(
    <Tooltip.Provider>
      <ExportPanel />
      <CanvasMenu>
        <div data-testid="stage" />
      </CanvasMenu>
    </Tooltip.Provider>,
  );
  openMenu('stage');
  await exportAs('edt-canvas-menu', 'png');
  await waitFor(() => expect(exportStore.get().menuBusy).toBe(true));
  const closing = projectStore.close();
  await screen.findByTestId('exp-panel-close-question');
  await done('01JEXP00000000000000000001', `${CRYPT}/exports/Crypt.png`);
  await closing;
  await waitFor(() => expect(projectStore.get().project).toBeNull());
  expect(screen.queryByTestId('exp-panel-close-ask')).toBeNull();
  expect(core.names()).not.toContain('export_cancel');
});
