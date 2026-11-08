// Mocked REF core commands (reference-packs §9), for Vitest and Playwright (KQ-33). It mirrors
// the real commands in pf-core `references/` (shapes final per R50): names, inputs, outputs,
// error codes, and the reconcile rules of `ref_resolve` (FR-REF-16..19).
//
// `refMock` must stay **self-contained** (no imports, no outer-scope references): Playwright
// injects it into the page with `fn.toString()` (e2e/ref-mock.ts). Type imports are erased.
// Production code never imports this file.
import type {
  DraftReferences,
  Pack,
  PackAddItem,
  PackBadge,
  PackCard,
  PackImageState,
  RefItem,
  RefNotice,
  RefResolved,
  RefRole,
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
  /** No `pack.json` could be written (FR-REF-04): `pack_id` null, `not_ready`. */
  notReady?: boolean;
  notices?: RefNotice[];
}

export interface RefMockSlot {
  name: string;
  role: RefRole | null;
  min: number;
  max: number;
  cost_affecting?: boolean;
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
  /** The open project is read-only (FR-REF-27). */
  readOnly?: boolean;
}

export interface RefMockEnv {
  emit(event: string, payload: unknown): void;
  /** False → `ref_resolve` fails with `no_project` (default: a project is open). */
  hasProject?(): boolean;
}

interface MockPack {
  pack_id: string;
  name: string;
  notes: string;
  images: RefMockImage[];
}

export interface RefMockState {
  packs: Map<string, MockPack>;
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
  const ROOT = `${seed.library}/References`;
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
  const addDays = (iso: string, days: number) =>
    new Date(Date.parse(iso) + days * 86_400_000).toISOString();

  const state: RefMockState = {
    packs: new Map(),
    trash: [...(seed.trash ?? [])],
    projectRefs: [...(seed.projectRefs ?? [])],
    addUserFile(packId, image) {
      state.packs.get(packId)?.images.push({ ...image });
    },
  };
  const extras = new Map<string, { badges: PackBadge[]; notices: RefNotice[] }>();
  const notReady: RefMockPack[] = [];
  for (const p of seed.packs) {
    if (p.notReady) {
      notReady.push(p);
      continue;
    }
    const id = p.pack_id ?? newId('PACK');
    state.packs.set(id, {
      pack_id: id,
      name: p.name,
      notes: p.notes ?? '',
      images: p.images.map((i) => ({ ...i })),
    });
    extras.set(id, { badges: p.badges ?? [], notices: p.notices ?? [] });
  }

  const taken = (name: string, except?: string) =>
    [...state.packs.values(), ...notReady.map((p) => ({ pack_id: '', name: p.name }))].some(
      (p) => p.pack_id !== except && lc(p.name) === lc(name),
    );
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
  const url = (key: string) => `http://pf-tiles.localhost/i/${key}.png`;
  const packOut = (p: MockPack): Pack => ({
    pack_id: p.pack_id,
    name: p.name,
    path: `${ROOT}/${p.name}`,
    notes: p.notes,
    images: sorted(p).map((i) => {
      const st = i.state ?? 'ok';
      return {
        file: i.file,
        path: `${ROOT}/${p.name}/${i.file}`,
        bytes: i.bytes ?? 1024,
        mtime: seed.now,
        ...(st === 'ok' ? { w: 256, h: 256, thumb_url: url(`ref-${p.pack_id}-${i.file}`) } : {}),
        role: i.role ?? null,
        state: st,
      };
    }),
  });
  const card = (p: MockPack): PackCard => {
    const local = p.images.filter((i) => (i.state ?? 'ok') === 'ok');
    const x = extras.get(p.pack_id);
    return {
      id: p.pack_id,
      pack_id: p.pack_id,
      name: p.name,
      path: `${ROOT}/${p.name}`,
      image_count: p.images.filter((i) => (i.state ?? 'ok') !== 'unsupported').length,
      notes_first_line: p.notes.split(/\r?\n/)[0] ?? '',
      thumb_url: local.length ? url(`pack-${p.pack_id}`) : null,
      badges: x?.badges ?? [],
      notices: x?.notices ?? [],
    };
  };
  const notReadyCard = (p: RefMockPack): PackCard => ({
    id: `${ROOT}/${p.name}`,
    pack_id: null,
    name: p.name,
    path: `${ROOT}/${p.name}`,
    image_count: p.images.length,
    notes_first_line: '',
    thumb_url: null,
    badges: ['not_ready', ...(p.badges ?? [])],
    notices: p.notices ?? [],
  });
  const getPack = (input: Rec) => state.packs.get(String(input.pack_id)) ?? null;
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

  const trashed = new Map<string, MockPack>();
  const trashItem = (kind: 'pack' | 'images', p: MockPack, files: string[]): RefTrashItem => {
    const item: RefTrashItem = {
      id: newId('TRASH'),
      kind,
      pack_id: p.pack_id,
      pack_name: p.name,
      files,
      trashed_at: seed.now,
      purge_at: addDays(seed.now, 30),
    };
    state.trash.unshift(item);
    return item;
  };

  // ── ref_resolve (FR-REF-16..19), mirroring pf-core `references::resolve::reconcile` ──────
  const resolve = (modelId: string, prev: DraftReferences | null): RefResolved | Promise<never> => {
    const slots = seed.slots?.[modelId];
    if (!slots) return fail('unknown_model', `Unknown model ${modelId}`);
    const readOnly = !!seed.readOnly;
    if (!slots.length)
      return {
        visible: false,
        slots: [],
        items: [],
        notices: [],
        references: prev ?? { packs: [], items: [] },
        ready: true,
        read_only: readOnly,
      };
    const notices: RefNotice[] = [];
    type Avail = Omit<RefItem, 'on' | 'role' | 'slot' | 'badges' | 'checkable'> & {
      packRole: RefRole | null;
    };
    const avail: Avail[] = [...state.projectRefs]
      .sort((a, b) => byName(a.file, b.file))
      .map((f) => ({
        key: `project:${f.file}`,
        source: 'project' as const,
        pack_id: null,
        pack_name: null,
        file: f.file,
        state: f.state ?? 'ok',
        packRole: null,
        ...((f.state ?? 'ok') === 'ok'
          ? { thumb_key: `ref-project-${f.file}`, thumb_url: url(`ref-project-${f.file}`) }
          : {}),
      }));
    const picked = (prev?.packs ?? []).map((want) => ({
      want,
      found:
        state.packs.get(want.pack_id) ??
        [...state.packs.values()].find((x) => lc(x.name) === lc(want.name)) ??
        null,
    }));
    for (const { found: p } of picked) {
      if (!p) continue;
      for (const i of sorted(p)) {
        const st = i.state ?? 'ok';
        if (st === 'unsupported') continue;
        const k = `ref-${p.pack_id}-${i.file}`;
        avail.push({
          key: `pack:${p.pack_id}:${i.file}`,
          source: 'pack',
          pack_id: p.pack_id,
          pack_name: p.name,
          file: i.file,
          state: st,
          packRole: i.role ?? null,
          ...(st === 'ok' ? { thumb_key: k, thumb_url: url(k) } : {}),
        });
      }
    }
    // A pack found by name under a new id keeps its items (FR-REF-26).
    const rekey = (k: string) => {
      for (const { want, found } of picked)
        if (found && found.pack_id !== want.pack_id && k.startsWith(`pack:${want.pack_id}:`))
          return `pack:${found.pack_id}:${k.slice(`pack:${want.pack_id}:`.length)}`;
      return k;
    };
    const prevItems = new Map((prev?.items ?? []).map((i) => [rekey(i.key), i]));
    const has = (r: RefRole | null) => r !== null && slots.some((s) => s.role === r);
    const slotIndex = (r: RefRole | null) =>
      Math.max(
        0,
        slots.findIndex((s) => r !== null && s.role === r),
      );
    const rows = avail.map(({ packRole, ...a }) => {
      const before = prevItems.get(a.key);
      const keptRole = before?.role ?? null;
      const role = has(keptRole) ? keptRole : has(packRole) ? packRole : (slots[0]!.role ?? null);
      const slot = slotIndex(role);
      const checkable = a.state === 'ok' || a.state === 'cloud_only';
      const badges: string[] = [];
      if (a.state === 'cloud_only') badges.push('cloud');
      if (a.state === 'corrupt') badges.push('corrupt');
      if (a.state === 'missing') badges.push('missing');
      const item: RefItem = {
        ...a,
        role,
        slot: slots[slot]!.name,
        on: false,
        badges,
        checkable,
      };
      return { item, slot, kept: before ? before.on && checkable : null };
    });
    const count = slots.map(() => 0);
    for (const r of rows)
      if (r.kept === true) {
        if (count[r.slot]! < slots[r.slot]!.max) {
          count[r.slot]!++;
          r.item.on = true;
        } else
          notices.push({
            code: 'limit_lowered',
            message: `${r.item.file} was unticked: ${slots[r.slot]!.name} takes at most ${slots[r.slot]!.max}`,
            key: r.item.key,
          });
      }
    for (const r of rows)
      if (r.kept === null && r.item.checkable && count[r.slot]! < slots[r.slot]!.max) {
        count[r.slot]!++;
        r.item.on = true;
      }
    const keys = new Set(avail.map((a) => a.key));
    const lost = picked.filter((p) => !p.found).map((p) => p.want);
    for (const p of lost)
      notices.push({ code: 'pack_unavailable', message: `The pack ${p.name} can't be found` });
    const missingRows: RefItem[] = [];
    const keptDraft: DraftReferences['items'] = [];
    for (const it of prev?.items ?? []) {
      const key = rekey(it.key);
      if (keys.has(key)) continue;
      const p = lost.find((l) => key.startsWith(`pack:${l.pack_id}:`));
      if (p) {
        missingRows.push({
          key,
          source: 'pack',
          pack_id: p.pack_id,
          pack_name: p.name,
          file: key.slice(`pack:${p.pack_id}:`.length),
          role: it.role,
          slot: slots[slotIndex(it.role)]!.name,
          on: false,
          state: 'missing',
          badges: ['missing'],
          checkable: false,
        });
        keptDraft.push({ ...it, key });
      } else
        notices.push({
          code: 'dropped',
          message: `${key.split(':').pop()} is no longer available`,
          key,
        });
    }
    const outSlots = slots.map((s, i) => ({
      name: s.name,
      role: s.role,
      min: s.min,
      max: s.max,
      cost_affecting: !!s.cost_affecting,
      on: count[i]!,
    }));
    const items = rows.map((r) => r.item);
    return {
      visible: true,
      slots: outSlots,
      items: [...items, ...missingRows],
      notices,
      references: {
        packs: picked.map(({ want, found }) =>
          found ? { pack_id: found.pack_id, name: found.name } : want,
        ),
        items: [...items.map((i) => ({ key: i.key, on: i.on, role: i.role })), ...keptDraft],
      },
      ready: outSlots.every((s) => s.on >= s.min),
      read_only: readOnly,
    };
  };

  const handlers: Record<string, Handler> = {
    ref_list_packs: () =>
      [...[...state.packs.values()].map(card), ...notReady.map(notReadyCard)].sort((a, b) =>
        byName(a.name, b.name),
      ),
    ref_pack_get: (input) => {
      const p = getPack(input);
      return p ? packOut(p) : fail('unknown_id', 'Unknown pack');
    },
    ref_pack_create: (input) => {
      const raw = String(input.name ?? '');
      if (!raw) return fail('invalid_input', 'the name is empty or too long', 'name');
      // A dropped folder's name is cleaned and suffixed; a typed one must already be valid.
      let name: string;
      if (input.suffix_if_taken === true) name = freeName(sanitize(raw) || 'untitled');
      else {
        name = raw;
        if (sanitize(name) !== name || !name)
          return fail('invalid_name', `“${name}” can’t be a folder name`, 'name');
        if (taken(name)) return fail('name_taken', `“${name}” is taken`, 'name');
      }
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
      const name = String(input.name ?? '');
      if (!name) return fail('invalid_input', 'the name is empty or too long', 'name');
      if (sanitize(name) !== name)
        return fail('invalid_name', `“${name}” can’t be a folder name`, 'name');
      if (taken(name, p.pack_id)) return fail('name_taken', `“${name}” is taken`, 'name');
      p.name = name;
      changed();
      return packOut(p);
    },
    ref_pack_update: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const notes = input.notes as string | undefined;
      if (notes !== undefined && notes !== null && notes.length > 2000)
        return fail('invalid_input', 'notes are at most 2000 characters', 'notes');
      const roles = (input.roles ?? {}) as Record<string, RefRole | null>;
      for (const [file, role] of Object.entries(roles)) {
        if (!p.images.some((i) => i.file === file))
          return fail('invalid_input', `${file} is not in the pack`, `roles.${file}`);
        if (role !== null && !ROLES.includes(role))
          return fail('invalid_input', `Unknown role ${String(role)}`, `roles.${file}`);
      }
      if (typeof notes === 'string') p.notes = notes;
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
      const paths = (input.paths as string[]) ?? [];
      if (!paths.length) return fail('invalid_input', 'choose at least one file', 'paths');
      const items = addTo(p, paths);
      changed();
      return { items, pack: packOut(p) };
    },
    ref_pack_remove_images: (input) => {
      const p = getPack(input);
      if (!p) return fail('unknown_id', 'Unknown pack');
      const want = (input.files as string[]) ?? [];
      if (!want.length) return fail('invalid_input', 'choose 1 to 10000 images', 'files');
      const files = want.filter((f) => p.images.some((i) => i.file === f));
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
      const id = String(input.id);
      const item = state.trash.find((t) => t.id === id);
      if (!item) return fail('unknown_id', 'Unknown trash item');
      const saved = trashed.get(id) ?? {
        pack_id: item.pack_id ?? newId('PACK'),
        name: item.pack_name,
        notes: '',
        images: item.files.map((file) => ({ file })),
      };
      let p: MockPack;
      if (item.kind === 'pack') {
        p = { ...saved, name: freeName(saved.name) };
        state.packs.set(p.pack_id, p);
      } else {
        const into = item.pack_id ? state.packs.get(item.pack_id) : undefined;
        if (!into) return fail('unknown_id', 'The pack is gone; restore it first');
        for (const img of saved.images)
          into.images.push({ ...img, file: freeFile(into, img.file) });
        p = into;
      }
      state.trash = state.trash.filter((t) => t.id !== id);
      changed();
      return { path: `${ROOT}/${p.name}` };
    },
    ref_resolve: (input) => {
      if (env.hasProject && !env.hasProject())
        return fail('no_project', 'no asset project is open');
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
