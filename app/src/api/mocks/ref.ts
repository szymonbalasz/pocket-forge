// Mocked REF core commands (reference-packs §9), for Vitest and Playwright (KQ-33). The core
// builder implements the same names and payloads in pf-core; this mock is the UI's contract with
// it. Shapes REF §9 leaves open are marked "assumed" in api/types.ts and the build report.
//
// `refMock` must stay **self-contained** (no imports, no outer-scope references): Playwright
// injects it into the page with `fn.toString()` (e2e/ref-mock.ts). Type imports are erased.
// Production code never imports this file.
import type {
  DraftReferences,
  PackAddItem,
  PackBadge,
  PackImageState,
  RefItem,
  RefRole,
  RefSlot,
  RefTrashItem,
  SkipReason,
} from '../types';

export interface RefMockImage {
  file: string;
  bytes?: number;
  role?: RefRole | null;
  state?: PackImageState;
  /** Content hash (default: the file name); equal hashes → `already_in_pack` (FR-REF-08). */
  hash?: string;
}

export interface RefMockPack {
  name: string;
  pack_id?: string;
  notes?: string;
  images: RefMockImage[];
  badges?: PackBadge[];
}

export interface RefMockSlot {
  name: string;
  role: RefRole | null;
  min: number;
  max: number;
}

export interface RefMockSeed {
  library: string;
  now: string;
  packs: RefMockPack[];
  trash?: RefTrashItem[];
  /** Source path → its content hash or the reason it is skipped (default: hash = path). */
  sources?: Record<string, { hash?: string; reason?: SkipReason }>;
  /** Folder path → the image paths found in it, recursively (a folder drop, FR-REF-07). */
  dirs?: Record<string, string[]>;
  /** The open project's `references/` images (FR-REF-16). */
  projectRefs?: { file: string; state?: 'ok' | 'cloud_only' | 'corrupt' }[];
  /** Model id → its reference slots (FR-PRV-31). Unknown model → `unknown_model`. */
  slots?: Record<string, RefMockSlot[]>;
}

export interface RefMockEnv {
  emit(event: string, payload: unknown): void;
  /** False → `ref_resolve` fails with `no_project` (default: a project is open). */
  hasProject?(): boolean;
}

export interface RefMockState {
  packs: Map<string, { pack_id: string; name: string; notes: string; images: RefMockImage[] }>;
  trash: RefTrashItem[];
  /** Simulates a file copied into a pack folder in Explorer (FR-REF-09). */
  addUserFile(packId: string, image: RefMockImage): void;
  /** The project's `references/` files (FR-REF-16), editable by a test. */
  projectRefs: { file: string; state?: 'ok' | 'cloud_only' | 'corrupt' }[];
}

type Rec = Record<string, unknown>;
type Handler = (input: Rec) => unknown;

export function refMock(
  seed: RefMockSeed,
  env: RefMockEnv,
): { handlers: Record<string, Handler>; state: RefMockState } {
  const IMAGE = /\.(png|jpe?g|webp)$/i;
  const RESERVED = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/i;
  const ROLES = ['style', 'structure', 'identity'];
  let nextId = 1;
  const newId = (tag: string) =>
    `01JMOCK${tag}${String(nextId++).padStart(26 - 7 - tag.length, '0')}`;
  const fail = (code: string, message: string, field?: string) =>
    Promise.reject(field ? { code, message, field } : { code, message });
  const lc = (s: string) => s.toLowerCase();
  const byName = (a: string, b: string) => (lc(a) < lc(b) ? -1 : lc(a) > lc(b) ? 1 : 0);
  const base = (p: string) =>
    p
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() ?? p;
  const sanitize = (raw: string): string => {
    let n = raw.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '-').replace(/[. ]+$/, '');
    if (RESERVED.test(n)) n += '_';
    return n.slice(0, 64);
  };

  type MockPack = RefMockState['packs'] extends Map<string, infer P> ? P : never;
  const state: RefMockState = {
    packs: new Map(),
    trash: [...(seed.trash ?? [])],
    projectRefs: [...(seed.projectRefs ?? [])],
    addUserFile(packId, image) {
      state.packs.get(packId)?.images.push({ ...image });
    },
  };
  for (const p of seed.packs) {
    const id = p.pack_id ?? newId('PACK');
    state.packs.set(id, {
      pack_id: id,
      name: p.name,
      notes: p.notes ?? '',
      images: p.images.map((i) => ({ ...i })),
    });
  }
  const badgesOf = new Map(seed.packs.map((p) => [p.pack_id ?? '', p.badges ?? []]));

  const taken = (name: string, except?: string) =>
    [...state.packs.values()].some((p) => p.pack_id !== except && lc(p.name) === lc(name));
  const freeName = (name: string) => {
    if (!taken(name)) return name;
    for (let n = 2; ; n++) if (!taken(`${name} (${n})`)) return `${name} (${n})`;
  };
  const freeFile = (p: MockPack, file: string) => {
    const has = (f: string) => p.images.some((i) => lc(i.file) === lc(f));
    if (!has(file)) return file;
    const dot = file.lastIndexOf('.');
    const [stem, ext] = dot > 0 ? [file.slice(0, dot), file.slice(dot)] : [file, ''];
    for (let n = 2; ; n++) if (!has(`${stem} (${n})${ext}`)) return `${stem} (${n})${ext}`;
  };
  const sorted = (p: MockPack) => [...p.images].sort((a, b) => byName(a.file, b.file));
  const thumb = (key: string) => `http://pf-tiles.localhost/i/${key}.png`;
  const packOut = (p: MockPack) => ({
    pack_id: p.pack_id,
    name: p.name,
    notes: p.notes,
    images: sorted(p).map((i) => ({
      file: i.file,
      bytes: i.bytes ?? 1024,
      mtime: seed.now,
      w: 256,
      h: 256,
      role: i.role ?? null,
      state: i.state ?? 'ok',
      thumb_url: (i.state ?? 'ok') === 'ok' ? thumb(`ref-${p.pack_id}-${i.file}`) : null,
    })),
  });
  const card = (p: MockPack) => {
    const local = p.images.filter((i) => (i.state ?? 'ok') === 'ok');
    return {
      pack_id: p.pack_id,
      name: p.name,
      image_count: p.images.filter((i) => (i.state ?? 'ok') !== 'unsupported').length,
      notes_line: p.notes.split(/\r?\n/)[0] ?? '',
      thumb_url: local.length ? `http://pf-tiles.localhost/i/pack-${p.pack_id}.jpg` : null,
      badges: badgesOf.get(p.pack_id) ?? [],
    };
  };
  const getPack = (input: Rec) => {
    const p = state.packs.get(String(input.pack_id));
    return p ?? null;
  };
  const changed = () => env.emit('ref:packs-changed', {});

  /** Copies sources into a pack (FR-REF-08): folders expand flat, unsupported files skip. */
  const addTo = (p: MockPack, paths: string[]): PackAddItem[] => {
    const files = paths.flatMap((src) => seed.dirs?.[src] ?? [src]);
    const items: PackAddItem[] = [];
    let done = 0;
    for (const source of files) {
      const meta = seed.sources?.[source] ?? {};
      done++;
      if (meta.reason) items.push({ status: 'skipped', source, reason: meta.reason });
      else if (!IMAGE.test(source))
        items.push({ status: 'skipped', source, reason: 'unsupported' });
      else {
        const hash = meta.hash ?? source;
        const dup = p.images.find((i) => (i.hash ?? i.file) === hash);
        if (dup) items.push({ status: 'already_in_pack', source, file: dup.file });
        else {
          const file = freeFile(p, base(source));
          p.images.push({ file, hash, bytes: 2048 });
          items.push({ status: 'added', source, file });
        }
      }
      env.emit('ref:add-progress', { done, total: files.length });
    }
    return items;
  };

  const trashItem = (kind: 'pack' | 'images', p: MockPack, files: string[]): RefTrashItem => {
    const item: RefTrashItem = {
      id: newId('TRASH'),
      kind,
      pack_id: p.pack_id,
      from: `References/${p.name}`,
      files,
      trashed_at: seed.now,
    };
    state.trash.unshift(item);
    return item;
  };
  const trashed = new Map<string, MockPack>();

  // ── ref_resolve (FR-REF-16..19) ───────────────────────────────────────────────────────────
  const resolve = (modelId: string, refs: DraftReferences | null) => {
    const slots = seed.slots?.[modelId];
    if (!slots) return fail('unknown_model', `Unknown model ${modelId}`);
    const notices: { code: string; message: string }[] = [];
    type Avail = Omit<RefItem, 'on' | 'role' | 'badges'> & {
      packRole: RefRole | null;
      badges: string[];
    };
    const avail: Avail[] = [...state.projectRefs]
      .sort((a, b) => byName(a.file, b.file))
      .map((f) => ({
        key: `project:${f.file}`,
        source: 'project' as const,
        file: f.file,
        state: f.state ?? 'ok',
        packRole: null,
        badges: [],
        thumb_url: (f.state ?? 'ok') === 'ok' ? thumb(`ref-project-${f.file}`) : null,
      }));
    for (const want of refs?.packs ?? []) {
      const p =
        state.packs.get(want.pack_id) ??
        [...state.packs.values()].find((x) => lc(x.name) === lc(want.name));
      if (!p) {
        notices.push({
          code: 'pack_missing',
          message: `Pack “${want.name}” is no longer available`,
        });
        continue;
      }
      for (const i of sorted(p)) {
        const st = i.state ?? 'ok';
        if (st === 'unsupported') continue;
        avail.push({
          key: `pack:${p.pack_id}:${i.file}`,
          source: 'pack',
          pack_id: p.pack_id,
          pack_name: p.name,
          file: i.file,
          state: st === 'cloud_only' ? 'cloud_only' : st === 'corrupt' ? 'corrupt' : 'ok',
          packRole: i.role ?? null,
          badges: [],
          thumb_url: st === 'ok' ? thumb(`ref-${p.pack_id}-${i.file}`) : null,
        });
      }
    }
    const prev = new Map((refs?.items ?? []).map((i) => [i.key, i]));
    const hasRole = (r: RefRole | null) => slots.some((s) => s.role === r);
    const slotOf = (r: RefRole | null | undefined) => slots.find((s) => s.role === r) ?? slots[0];
    const count = new Map<string, number>();
    const items: RefItem[] = avail.map(({ packRole, ...a }) => {
      const before = prev.get(a.key);
      const role: RefRole | null =
        before && hasRole(before.role)
          ? before.role
          : packRole && hasRole(packRole)
            ? packRole
            : (slots[0]?.role ?? null);
      const slot = slotOf(role);
      const n = count.get(slot?.name ?? '') ?? 0;
      const usable = a.state === 'ok' || a.state === 'cloud_only';
      let on = usable && (before ? before.on : n < (slot?.max ?? 0));
      if (on && n >= (slot?.max ?? 0)) {
        on = false;
        notices.push({ code: 'limit_lowered', message: `${a.file} was unticked: limit reached` });
      }
      if (on) count.set(slot?.name ?? '', n + 1);
      const badges = [...a.badges];
      if (a.state === 'cloud_only') badges.push('cloud');
      if (a.state === 'corrupt') badges.push('corrupt');
      return { ...a, role, on, badges };
    });
    for (const k of prev.keys())
      if (!items.some((i) => i.key === k))
        notices.push({ code: 'dropped', message: `${k.split(':').pop()} is no longer available` });
    const outSlots: RefSlot[] = slots.map((s) => ({ ...s, on: count.get(s.name) ?? 0 }));
    return { slots: outSlots, items, notices };
  };

  const handlers: Record<string, Handler> = {
    ref_list_packs: () =>
      [...state.packs.values()].sort((a, b) => byName(a.name, b.name)).map(card),
    ref_pack_get: (input) => {
      const p = getPack(input);
      return p ? packOut(p) : fail('unknown_id', 'Unknown pack');
    },
    ref_pack_create: (input) => {
      const raw = String(input.name ?? '').trim();
      const name = sanitize(raw);
      if (!name) return fail('invalid_name', 'Enter a name', 'name');
      if (taken(name)) return fail('name_taken', `“${name}” is taken`, 'name');
      const id = newId('PACK');
      const p: MockPack = { pack_id: id, name, notes: '', images: [] };
      state.packs.set(id, p);
      const items = addTo(p, (input.paths as string[] | undefined) ?? []);
      changed();
      return { pack: packOut(p), items };
    },
    ref_pack_rename: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const name = sanitize(String(input.name ?? '').trim());
      if (!name) return fail('invalid_name', 'Enter a name', 'name');
      if (taken(name, p.pack_id)) return fail('name_taken', `“${name}” is taken`, 'name');
      p.name = name;
      changed();
      return packOut(p);
    },
    ref_pack_update: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const notes = input.notes as string | undefined;
      if (notes !== undefined && notes.length > 2000)
        return fail('invalid_input', 'Notes are at most 2000 characters', 'notes');
      const roles = (input.roles ?? {}) as Record<string, RefRole | null>;
      for (const [file, role] of Object.entries(roles)) {
        if (!p.images.some((i) => i.file === file))
          return fail('invalid_input', `${file} is not in the pack`, 'roles');
        if (role !== null && !ROLES.includes(role))
          return fail('invalid_input', `Unknown role ${String(role)}`, 'roles');
      }
      if (notes !== undefined) p.notes = notes;
      for (const [file, role] of Object.entries(roles)) {
        const img = p.images.find((i) => i.file === file);
        if (img) img.role = role;
      }
      changed();
      return packOut(p);
    },
    ref_pack_add_images: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const items = addTo(p, (input.paths as string[]) ?? []);
      changed();
      return { items };
    },
    ref_pack_remove_images: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const files = ((input.files as string[]) ?? []).filter((f) =>
        p.images.some((i) => i.file === f),
      );
      const removed = p.images.filter((i) => files.includes(i.file));
      p.images = p.images.filter((i) => !files.includes(i.file));
      const item = trashItem('images', p, files);
      trashed.set(item.id, { ...p, images: removed });
      changed();
      return { item_id: item.id };
    },
    ref_pack_delete: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      state.packs.delete(p.pack_id);
      const item = trashItem(
        'pack',
        p,
        p.images.map((i) => i.file),
      );
      trashed.set(item.id, p);
      changed();
      return { item_id: item.id };
    },
    ref_trash_list: () => [...state.trash],
    ref_trash_restore: (input) => {
      const id = String(input.item_id);
      const item = state.trash.find((t) => t.id === id);
      if (!item) return fail('unknown_id', 'Unknown trash item');
      state.trash = state.trash.filter((t) => t.id !== id);
      const saved = trashed.get(id) ?? {
        pack_id: item.pack_id,
        name: item.from.replace(/^References\//, ''),
        notes: '',
        images: item.files.map((file) => ({ file })),
      };
      let path: string;
      if (item.kind === 'pack') {
        const p = { ...saved, name: freeName(saved.name) };
        state.packs.set(p.pack_id, p);
        path = `References/${p.name}`;
      } else {
        const p = state.packs.get(item.pack_id);
        if (!p) return fail('unknown_id', 'The pack is gone; restore it first');
        for (const img of saved.images) p.images.push({ ...img, file: freeFile(p, img.file) });
        path = `References/${p.name}`;
      }
      changed();
      return { path };
    },
    ref_resolve: (input) => {
      if (env.hasProject && !env.hasProject()) return fail('no_project', 'No project is open');
      return resolve(String(input.model_id), (input.references as DraftReferences | null) ?? null);
    },
  };
  return { handlers, state };
}

/** A small public-safe seed: one pack with three swatches, one empty pack. */
export function sampleRefSeed(library = 'C:/pf-mock/library'): RefMockSeed {
  return {
    library,
    now: '2026-11-08T12:00:00.000Z',
    packs: [
      {
        name: 'Weapons',
        pack_id: '01JMOCKPACKWEAPONS00000000',
        notes: 'Swords and axes\nsecond line',
        images: [
          { file: 'swatch-a.png', role: 'style' },
          { file: 'swatch-b.jpg' },
          { file: 'swatch-c.webp', state: 'cloud_only' },
        ],
      },
      { name: 'armour', pack_id: '01JMOCKPACKARMOUR000000000', images: [] },
    ],
    dirs: {
      'D:/art/Crypt moods': ['D:/art/Crypt moods/fog.png', 'D:/art/Crypt moods/sub/bones.webp'],
    },
    sources: { 'D:/art/truncated.png': { reason: 'corrupt' } },
  };
}
