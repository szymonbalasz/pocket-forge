// Pure helpers for the packs UI and the References block (reference-packs §7). No React here.
import type { PackAddItem, RefItem, RefResolved, RefRole, RefSlot, SkipReason } from '../../api';

const IMAGE = /\.(png|jpe?g|webp)$/i;
const ABSOLUTE = /^(?:[A-Za-z]:[\\/]|\\\\|\/)/;

/** True for a path with a supported image extension (FR-REF-01); other drops are folders. */
export function isImagePath(path: string): boolean {
  return IMAGE.test(path);
}

/** The last segment of a path (the dropped folder's name, FR-REF-07). */
export function folderName(path: string): string {
  return (
    path
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || path
  );
}

/** The absolute paths in pasted text (Explorer's "Copy as path", `file://` URIs), or []. */
export function parsePastedPaths(text: string): string[] {
  const paths = text
    .split(/\r?\n/)
    .map((l) => l.trim().replace(/^"(.*)"$/, '$1'))
    .filter(Boolean)
    .map((l) => {
      if (!/^file:\/\//i.test(l)) return l;
      try {
        const p = decodeURIComponent(new URL(l).pathname);
        return /^\/[A-Za-z]:\//.test(p) ? p.slice(1) : p;
      } catch {
        return l;
      }
    });
  return paths.length > 0 && paths.every((p) => ABSOLUTE.test(p)) ? paths : [];
}

/** True while a text field has focus: paste and shortcuts belong to it. */
export function isTextTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    (target instanceof HTMLInputElement && !['button', 'checkbox', 'radio'].includes(target.type))
  );
}

const REASON: Record<SkipReason, string> = {
  unsupported: 'unsupported',
  animated: 'animated',
  too_large: 'too large',
  corrupt: 'corrupt',
  not_found: 'not found',
  io: 'read error',
  not_available_offline: 'not available offline',
};

export function reasonLabel(reason: SkipReason): string {
  return REASON[reason] ?? String(reason).replace(/_/g, ' ');
}

/** "2 added · 1 already in pack · 1 skipped (corrupt)" (FR-REF-08). */
export function summariseAdd(items: PackAddItem[]): string {
  const added = items.filter((i) => i.status === 'added').length;
  const already = items.filter((i) => i.status === 'already_in_pack').length;
  const reasons = items.flatMap((i) => (i.status === 'skipped' ? [i.reason] : []));
  const parts = [];
  if (added) parts.push(`${added} added`);
  if (already) parts.push(`${already} already in pack`);
  if (reasons.length)
    parts.push(`${reasons.length} skipped (${[...new Set(reasons)].map(reasonLabel).join(', ')})`);
  return parts.join(' · ') || 'No images found';
}

export function imagesLabel(n: number): string {
  return n === 1 ? '1 image' : `${n} images`;
}

/** A Recently deleted row: the pack, its images, or (not restorable, R61) its `pack.json`. */
export function trashLabel(t: {
  kind: 'pack' | 'images';
  pack_name: string;
  files: string[];
  restorable?: boolean;
}): string {
  if (t.kind === 'pack') return t.pack_name;
  if (t.restorable === false)
    return t.files.length === 1
      ? `${t.pack_name}’s older pack.json`
      : `${t.files.length} older pack.json copies of ${t.pack_name}`;
  return `${imagesLabel(t.files.length)} from ${t.pack_name}`;
}

export const ROLES: readonly RefRole[] = ['style', 'structure', 'identity'];

export function roleLabel(role: RefRole | null | undefined): string {
  return role ? role[0]!.toUpperCase() + role.slice(1) : 'No role';
}

// ── References block (FR-REF-15..19) ─────────────────────────────────────────────────────────

/** The slot an item counts against: the core's `slot`, else the slot with its role, else the
 *  first (FR-REF-17). */
export function slotOf(
  slots: readonly RefSlot[],
  item: Pick<RefItem, 'role'> & { slot?: string },
): RefSlot | undefined {
  return (
    slots.find((s) => s.name === item.slot) ??
    slots.find((s) => s.role !== null && s.role === (item.role ?? null)) ??
    slots[0]
  );
}

/** FR-REF-19: `none` one slot without a role · `label` one slot with a role · `select` several. */
export function roleMode(slots: readonly RefSlot[]): 'none' | 'label' | 'select' {
  if (slots.length > 1) return 'select';
  return slots[0]?.role ? 'label' : 'none';
}

/** The header count: `References 3 / 4`, or `Style 2/4 · Identity 1/1` with several slots. */
export function headerCount(slots: readonly RefSlot[]): string {
  if (slots.length === 1) return `References ${slots[0]!.on} / ${slots[0]!.max}`;
  return slots.map((s) => `${slotName(s)} ${s.on}/${s.max}`).join(' · ');
}

/** A slot as the block names it: its role, else "References" (never the provider's input
 *  name; pf-core `slot_label`). */
export function slotName(slot: Pick<RefSlot, 'role'>): string {
  return slot.role ? roleLabel(slot.role) : 'References';
}

/** Items that can be ticked at all (corrupt or missing ones can't, §11). */
export function tickable(item: RefItem): boolean {
  return item.checkable;
}

/** FR-REF-18: ticking is refused at the slot's max ("limit N reached"); null = allowed. */
export function tickBlock(slots: readonly RefSlot[], item: RefItem): string | null {
  if (item.on) return null;
  if (!tickable(item)) return item.state === 'missing' ? 'missing' : 'corrupt';
  const slot = slotOf(slots, item);
  if (!slot) return 'no slot';
  return slot.on >= slot.max ? `limit ${slot.max} reached` : null;
}

/** "6 available · 4 sent" when more items are available than are sent (§7 over the limit). */
export function overLimit(r: RefResolved): string | null {
  const available = r.items.filter(tickable).length;
  const sent = r.items.filter((i) => i.on).length;
  const cap = r.slots.reduce((n, s) => n + s.max, 0);
  return available > cap ? `${available} available · ${sent} sent` : null;
}

/** Slots under their `min` (FR-REF-18 `references_required(slot, min)`): submit is disabled. */
export function underMin(slots: readonly RefSlot[]): RefSlot[] {
  return slots.filter((s) => s.on < s.min);
}

/** A badge's text: `changed_since:003` → "changed since 003". */
export function badgeLabel(badge: string): string {
  const [code, arg] = badge.split(':');
  const text = (code ?? badge).replace(/_/g, ' ');
  return arg ? `${text} ${arg}` : text;
}
