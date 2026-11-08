// Mocked PRJ core commands (asset-project §9), for Vitest and Playwright (KQ-33). The core builder
// implements the same names and payloads in pf-core; this mock is the UI's contract with it.
//
// `prjMock` must stay **self-contained** (no imports, no outer-scope references): Playwright
// injects it into the page with `fn.toString()` (e2e/mock-core.ts). Types are erased, so type
// imports are fine. Production code never imports this file.
import type {
  CardBadge,
  CreateItem,
  DiskUsage,
  FolderFile,
  ProjectCard,
  ProjectFolder,
  ReadOnlyReason,
  RevisionEntry,
  Section,
  TrashItem,
} from '../types';

export interface PrjMockProject {
  section: Section;
  name: string;
  iterations?: number;
  last_edited: string;
  badges?: CardBadge[];
  /** Original file name in `working/` (default `<name>.png`). */
  original?: string;
  read_only?: ReadOnlyReason | null;
  warnings?: { code: string; message: string }[];
  folders?: Partial<Record<ProjectFolder, FolderFile[]>>;
  trash?: TrashItem[];
  /** No thumbnail yet (`thumb_url: null`). */
  noThumb?: boolean;
  /** `project.json` unreadable: the card has no `last_edited` and the `unreadable` badge (R45). */
  unreadable?: boolean;
  /** A save still pending after `locked`: `project_close` answers `unsaved` (FR-PRJ-21). */
  pendingSave?: boolean;
}

export interface PrjMockSeed {
  library: string;
  configured: boolean;
  /** The root is offline / missing at startup (FR-PRJ-02). */
  unavailable?: boolean;
  image: { w: number; h: number };
  projects: PrjMockProject[];
  /** Mock clock (FR-PRJ-10 `pasted-<UTC yyyymmdd-hhmmss>`, `mtime`s). */
  now: string;
  /** Folder path → the image paths found in it, recursively (FR-PRJ-07). */
  dirs?: Record<string, string[]>;
  /** Delay per created file, so a test can cancel mid-batch (FR-PRJ-11). */
  createDelayMs?: number;
}

export interface PrjMockEnv {
  /** Emits a core event to the UI. */
  emit(event: string, payload: unknown): void;
  /** Extra snapshot fields a feature mock forces (`window.__pfMockSnapshot` in e2e). */
  snapshotOverride?(): Record<string, unknown> | undefined;
}

type Rec = Record<string, unknown>;
type Handler = (input: Rec) => unknown;

export interface PrjMockState {
  config: { configured: boolean; unavailable: boolean; root: string };
  projects: Map<string, Rec>;
  openPath: string | null;
  /** Simulates a file dropped into a project folder in Explorer (FR-PRJ-48). */
  addUserFile(projectPath: string, folder: ProjectFolder, file: FolderFile): void;
  /** Makes the root (un)available, as a disconnected drive would (FR-PRJ-02). */
  setUnavailable(on: boolean): void;
}

export function prjMock(
  seed: PrjMockSeed,
  env: PrjMockEnv,
): { handlers: Record<string, Handler>; state: PrjMockState } {
  const SECTION_DIR: Record<string, string> = { maps: 'Maps', tokens: 'Tokens', items: 'Items' };
  const KIND: Record<string, string> = { maps: 'map', tokens: 'token', items: 'item' };
  const FOLDERS = ['working', 'references', 'structure', 'parts', 'exports'];
  const IMAGE = /\.(png|jpe?g|webp)$/i;
  const fail = (code: string, message: string, extra: Rec = {}) =>
    Promise.reject({ code, message, ...extra });
  const base = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const stem = (p: string) => base(p).replace(/\.[^.]*$/, '');
  let ids = 0;
  const nextId = () => `01JMOCKPRJ${String(++ids).padStart(16, '0')}`;
  let revIds = 0;
  const nextRev = () => `01JMOCKREV${String(++revIds).padStart(16, '0')}`;

  // FR-PRJ-44 name rules (the real ones are pf-core's; this is close enough for the UI).
  const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
  const sanitize = (raw: string): string => {
    let n = raw.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '-').replace(/[. ]+$/, '');
    if (RESERVED.test(n)) n += '_';
    n = n.slice(0, 64);
    return n || 'untitled';
  };

  const state: PrjMockState = {
    config: { configured: seed.configured, unavailable: !!seed.unavailable, root: seed.library },
    projects: new Map(),
    openPath: null,
    addUserFile(projectPath, folder, file) {
      const p = state.projects.get(projectPath);
      if (!p) return;
      const folders = p.folders as Record<string, FolderFile[]>;
      (folders[folder] ??= []).push(file);
    },
    setUnavailable(on) {
      state.config.unavailable = on;
    },
  };

  const sectionDir = (section: string) => `${state.config.root}/${SECTION_DIR[section]}`;
  const projectPath = (section: string, name: string) => `${sectionDir(section)}/${name}`;
  const taken = (section: string, name: string, except?: string) =>
    [...state.projects.values()].some(
      (p) =>
        p.section === section &&
        p.path !== except &&
        String(p.name).toLowerCase() === name.toLowerCase(),
    );
  const freeName = (section: string, wanted: string) => {
    if (!taken(section, wanted)) return wanted;
    for (let i = 2; ; i++) {
      const n = `${wanted} (${i})`;
      if (!taken(section, n)) return n;
    }
  };

  const addProject = (p: PrjMockProject & { source?: string }): Rec => {
    const id = nextId();
    const path = projectPath(p.section, p.name);
    const original = p.original ?? `${p.name}.png`;
    const rec: Rec = {
      id,
      path,
      section: p.section,
      name: p.name,
      iterations: p.iterations ?? 0,
      last_edited: p.last_edited,
      badges: p.badges ?? [],
      thumb: !p.noThumb,
      unreadable: !!p.unreadable,
      original,
      source: p.source ?? null,
      read_only: p.read_only ?? null,
      warnings: p.warnings ?? [],
      folders: {
        working: [{ name: original, bytes: 4096, mtime: p.last_edited, image: true }],
        references: [],
        structure: [],
        parts: [],
        exports: [],
        ...(p.folders ?? {}),
      },
      trash: p.trash ?? [],
      pendingSave: !!p.pendingSave,
      revisions: [
        { id: nextRev(), seq: 0, created_at: p.last_edited, reason: 'create' },
      ] as RevisionEntry[],
      dirty: false,
      seq: 1,
    };
    state.projects.set(path, rec);
    return rec;
  };
  for (const p of seed.projects) addProject(p);

  const card = (p: Rec): ProjectCard => ({
    id: String(p.id),
    path: String(p.path),
    name: String(p.name),
    kind: KIND[String(p.section)] as ProjectCard['kind'],
    iterations: Number(p.iterations),
    last_edited: p.unreadable ? null : String(p.last_edited),
    thumb_url: p.thumb ? `http://pf-tiles.localhost/i/lib-${String(p.id)}.jpg` : null,
    badges: p.unreadable ? [...(p.badges as CardBadge[]), 'unreadable'] : (p.badges as CardBadge[]),
  });

  const open = (): Rec | null =>
    state.openPath ? (state.projects.get(state.openPath) ?? null) : null;

  const snapshot = (p: Rec): Rec => {
    const revisions = p.revisions as RevisionEntry[];
    return {
      project_id: p.id,
      kind: KIND[String(p.section)],
      name: p.name,
      image_path: `${String(p.path)}/working/${String(p.original)}`,
      project_dir: p.path,
      saved: true,
      read_only: p.read_only,
      warnings: [...(p.warnings as Rec[])],
      width: seed.image.w,
      height: seed.image.h,
      format: 'png',
      layers: [
        {
          id: '01JLAYER00000000000000000',
          kind: 'original',
          file: null,
          cx: seed.image.w / 2,
          cy: seed.image.h / 2,
          scale: 1,
          rotation: 0,
          opacity: 1,
          visible: true,
        },
      ],
      draft: null,
      revisions: [...revisions],
      head_revision: revisions.at(-1)?.id ?? null,
      commit_seq: p.seq,
      committed_at: p.last_edited,
      ...(env.snapshotOverride?.() ?? {}),
    };
  };

  const pad = (n: number) => String(n).padStart(2, '0');
  const stamp = () => {
    const d = new Date(seed.now);
    return `${d.getUTCFullYear()}${pad(d.getUTCMonth() + 1)}${pad(d.getUTCDate())}-${pad(d.getUTCHours())}${pad(d.getUTCMinutes())}${pad(d.getUTCSeconds())}`;
  };
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  let canceled = false;

  const createOne = (section: string, source: string): CreateItem => {
    const name = base(source);
    if (/corrupt/i.test(name)) return { status: 'skipped', source, reason: 'corrupt' };
    if (/animated/i.test(name)) return { status: 'skipped', source, reason: 'animated' };
    if (/huge/i.test(name)) return { status: 'skipped', source, reason: 'too_large' };
    if (!IMAGE.test(name)) return { status: 'skipped', source, reason: 'unsupported' };
    const dup = [...state.projects.values()].find(
      (p) => p.section === section && p.source === source,
    );
    if (dup) return { status: 'already_in_library', source, path: String(dup.path) };
    const rec = addProject({
      section: section as Section,
      name: freeName(section, sanitize(stem(name))),
      original: name,
      last_edited: seed.now,
      source,
    });
    return { status: 'created', source, path: String(rec.path) };
  };

  const handlers: Record<string, Handler> = {
    library_get_config: () => ({
      roots: state.config.configured
        ? [{ path: state.config.root, available: !state.config.unavailable }]
        : [],
      overridden: false,
      configured: state.config.configured,
    }),
    library_set_roots: (input) => {
      const roots = (input.roots as string[]) ?? [];
      const root = (roots[0] ?? '').trim();
      if (!root) return fail('invalid_root', 'Pick a folder', { field: 'roots[0]' });
      if (/missing/i.test(root))
        return fail('not_found', 'That folder does not exist', { field: 'roots[0]' });
      state.config = { configured: true, unavailable: false, root };
      return handlers.library_get_config!({});
    },
    library_list_projects: (input) => {
      if (state.config.unavailable) return fail('library_unavailable', 'Library unavailable');
      return (
        [...state.projects.values()]
          .filter((p) => p.section === input.section)
          .map(card)
          // Newest first; cards without a time (unreadable) last.
          .sort((a, b) => (b.last_edited ?? '').localeCompare(a.last_edited ?? ''))
      );
    },
    project_create: async (input) => {
      if (state.config.unavailable) return fail('library_unavailable', 'Library unavailable');
      const section = String(input.section);
      const sources = (input.paths as string[]).flatMap((p) => seed.dirs?.[p] ?? [p]);
      const items: CreateItem[] = [];
      canceled = false;
      for (const [i, source] of sources.entries()) {
        if (seed.createDelayMs) await sleep(seed.createDelayMs);
        if (canceled) return fail('canceled', 'Creation canceled', { detail: { items } });
        items.push(createOne(section, source));
        env.emit('import:progress', { done: i + 1, total: sources.length });
      }
      env.emit('import:done', { total: sources.length });
      return { items };
    },
    project_create_cancel: () => {
      canceled = true;
      return {};
    },
    project_create_from_clipboard: (input) => {
      const section = String(input.section);
      if (!input.data_b64) return fail('unsupported', 'The clipboard holds no image');
      const name = freeName(section, `pasted-${stamp()}`);
      const rec = addProject({
        section: section as Section,
        name,
        original: `${name}.png`,
        last_edited: seed.now,
      });
      return { path: rec.path };
    },
    project_rename: (input) => {
      const path = String(input.path);
      const p = state.projects.get(path);
      if (!p) return fail('not_found', 'Project not found');
      if (state.openPath === path) return fail('project_open', 'Close the project to rename it');
      const raw = String(input.name).trim();
      if (!raw) return fail('invalid_name', 'Enter a name');
      const name = sanitize(raw);
      if (taken(String(p.section), name, path)) return fail('name_taken', `“${name}” is taken`);
      const next = projectPath(String(p.section), name);
      state.projects.delete(path);
      p.name = name;
      p.path = next;
      state.projects.set(next, p);
      return { path: next };
    },
    project_open: (input) => {
      const path = String(input.path);
      const p = state.projects.get(path);
      if (!p) return fail('not_found', 'Project not found');
      if (state.openPath) state.openPath = null;
      state.openPath = path;
      return snapshot(p);
    },
    project_get: () => {
      const p = open();
      return p ? snapshot(p) : null;
    },
    project_close: (input) => {
      const p = open();
      if (p?.pendingSave && !input.force)
        return fail('unsaved', 'A save is still pending (the file is locked); close anyway?');
      state.openPath = null;
      return {};
    },
    project_put_draft: () => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      if (p.read_only) return fail('read_only', 'Read-only');
      p.dirty = true;
      p.seq = Number(p.seq) + 1;
      return { seq: p.seq };
    },
    project_save_revision: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      if (p.read_only) return fail('read_only', 'Read-only');
      const label = typeof input.label === 'string' && input.label ? input.label : undefined;
      if (!p.dirty && !label) return fail('unchanged', 'Nothing changed since the last revision');
      const revs = p.revisions as RevisionEntry[];
      const rev: RevisionEntry = {
        id: nextRev(),
        seq: revs.length,
        created_at: seed.now,
        reason: 'save',
        ...(label ? { label } : {}),
      };
      revs.push(rev);
      p.dirty = false;
      p.last_edited = seed.now;
      p.seq = Number(p.seq) + 1;
      return rev;
    },
    project_list_revisions: () => {
      const p = open();
      return p ? [...(p.revisions as RevisionEntry[])] : [];
    },
    project_restore_revision: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      if (p.read_only) return fail('read_only', 'Read-only');
      const revs = p.revisions as RevisionEntry[];
      if (!revs.some((r) => r.id === input.rev_id)) return fail('unknown_id', 'Unknown revision');
      const rev: RevisionEntry = {
        id: nextRev(),
        seq: revs.length,
        created_at: seed.now,
        reason: 'restore',
      };
      revs.push(rev);
      p.seq = Number(p.seq) + 1;
      return rev;
    },
    project_folder_list: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      const folder = String(input.folder);
      if (!FOLDERS.includes(folder)) return fail('unknown_folder', `Unknown folder ${folder}`);
      return [...((p.folders as Record<string, FolderFile[]>)[folder] ?? [])];
    },
    project_folder_add: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      const folder = String(input.folder);
      if (!FOLDERS.includes(folder)) return fail('unknown_folder', `Unknown folder ${folder}`);
      if (p.read_only) return fail('read_only', 'Read-only');
      for (const path of input.paths as string[])
        state.addUserFile(String(p.path), folder as ProjectFolder, {
          name: base(path),
          bytes: 2048,
          mtime: seed.now,
          image: IMAGE.test(path),
        });
      return [...((p.folders as Record<string, FolderFile[]>)[folder] ?? [])];
    },
    project_trash_list: () => [...((open()?.trash as TrashItem[]) ?? [])],
    project_trash_restore: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      if (p.read_only) return fail('read_only', 'Read-only');
      const trash = p.trash as TrashItem[];
      const i = trash.findIndex((t) => t.id === input.item_id);
      if (i < 0) return fail('unknown_id', 'Unknown trash item');
      trash.splice(i, 1);
      return {};
    },
    project_disk_usage: (): DiskUsage => {
      const p = open();
      const trash = (p?.trash as TrashItem[]) ?? [];
      const sum = (f: string) =>
        ((p?.folders as Record<string, FolderFile[]>)?.[f] ?? []).reduce((s, x) => s + x.bytes, 0);
      const oldest = trash.map((t) => t.trashed_at).sort()[0];
      return {
        bytes: {
          original: sum('working'),
          layers: 0,
          masks: 0,
          job_inputs: 0,
          results: 0,
          trash: trash.reduce((s, t) => s + t.bytes, 0),
          user_folders: sum('references') + sum('structure') + sum('parts'),
          exports: sum('exports'),
          other: 0,
        },
        trash_count: trash.length,
        next_purge: oldest
          ? new Date(new Date(oldest).getTime() + 30 * 86400_000).toISOString()
          : null,
      };
    },
    project_relink_original: (input) => {
      const p = open();
      if (!p) return fail('no_project', 'No asset project is open');
      if (base(String(input.path)) !== p.original)
        return fail('hash_mismatch', 'That file is not the original');
      p.warnings = (p.warnings as Rec[]).filter((w) => w.code !== 'original_missing');
      if (p.read_only === 'original_missing') p.read_only = null;
      return {};
    },
  };

  return { handlers, state };
}

/** A small public-safe library: two maps, a token, nothing in Items. */
export function samplePrjSeed(library = 'C:/pf-mock/library'): PrjMockSeed {
  return {
    library,
    configured: true,
    image: { w: 512, h: 640 },
    now: '2026-11-08T12:00:00.000Z',
    projects: [
      { section: 'maps', name: 'Crypt', iterations: 3, last_edited: '2026-11-07T10:00:00.000Z' },
      {
        section: 'maps',
        name: 'Harbour',
        iterations: 1,
        last_edited: '2026-11-08T09:00:00.000Z',
        badges: ['read_only'],
        noThumb: true,
      },
      { section: 'tokens', name: 'Goblin', last_edited: '2026-11-06T08:00:00.000Z' },
    ],
    dirs: {
      'D:/art/tokens': ['D:/art/tokens/orc.png', 'D:/art/tokens/sub/elf.webp'],
    },
  };
}
