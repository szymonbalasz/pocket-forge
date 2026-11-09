// Pure rules of the prompt block and the helper settings (FR-PRM-01, 04, 05, 07, 10, 11).
import type { HelperSkipReason, ProjectKind, PromptHelper } from '../../api';

/** Who renders the prompt block (FR-PRM-43): Region edit (RGN) or New image (GEN). */
export type PromptHost = 'rgn' | 'gen';

export const MAX_HELPERS = 60;
export const MAX_LABEL_CHARS = 40;
export const MAX_TEXT_CHARS = 500;

/** FR-PRM-11: the empty prompt field's placeholder per host. */
export const PLACEHOLDER: Record<PromptHost, string> = {
  rgn: "Describe the result, e.g. 'a kitchen with a stone floor and a hearth'",
  gen: 'Describe the image',
};

/** FR-PRM-11: the Tips popover, fixed; the first one is the spec's. */
export const TIPS: readonly string[] = [
  'Describe the result, not an instruction: “a kitchen with a stone floor”, not “add a kitchen”.',
  'Name the main subject first, then materials, lighting and mood.',
  'Leave out what a ticked helper already says: its text is added for you.',
];

export const KIND_TABS: readonly { kind: ProjectKind; label: string }[] = [
  { kind: 'map', label: 'Maps' },
  { kind: 'token', label: 'Tokens' },
  { kind: 'item', label: 'Items' },
];

/** FR-PRM-04: the chips of the helpers row, in settings order. */
export function rowHelpers(helpers: readonly PromptHelper[], kind: ProjectKind): PromptHelper[] {
  return helpers.filter((h) => !h.hidden && h.kinds.includes(kind));
}

/** FR-PRM-05: the ticked ids; `null` ticks = every row helper with `default_on`. */
export function effectiveTicks(
  ticks: readonly string[] | null,
  helpers: readonly PromptHelper[],
  kind: ProjectKind,
): string[] {
  if (ticks) return [...ticks];
  return rowHelpers(helpers, kind)
    .filter((h) => h.default_on)
    .map((h) => h.id);
}

/** FR-PRM-05: a tick change always writes the explicit list. */
export function toggleTick(
  ticks: readonly string[] | null,
  helpers: readonly PromptHelper[],
  kind: ProjectKind,
  id: string,
): string[] {
  const now = effectiveTicks(ticks, helpers, kind);
  return now.includes(id) ? now.filter((t) => t !== id) : [...now, id];
}

/** FR-PRM-07: the reason a chip shows for a skipped helper. */
export function skipLabel(reason: HelperSkipReason): string {
  return reason === 'duplicate' ? 'Duplicate' : 'In blend instruction';
}

export function skipTitle(reason: HelperSkipReason): string {
  return reason === 'duplicate'
    ? 'Not sent: another ticked helper says the same'
    : 'Not sent: the blend instruction already says this';
}

/** Characters as the core counts them (Unicode scalar values). */
export const charCount = (s: string): number => [...s].length;

/** FR-PRM-01: the first problem with a helper's fields, or null. */
export function fieldError(h: {
  label: string;
  text: string;
  kinds: readonly ProjectKind[];
}): { field: 'label' | 'text' | 'kinds'; message: string } | null {
  const label = charCount(h.label.trim());
  if (label < 1 || label > MAX_LABEL_CHARS)
    return { field: 'label', message: `A label is 1 to ${MAX_LABEL_CHARS} characters` };
  const text = charCount(h.text.trim());
  if (text < 1 || text > MAX_TEXT_CHARS)
    return { field: 'text', message: `A text is 1 to ${MAX_TEXT_CHARS} characters` };
  if (h.kinds.length === 0) return { field: 'kinds', message: 'Pick at least one asset type' };
  return null;
}

/** Message for a `too_many` save (FR-PRM-01). */
export const TOO_MANY = `At most ${MAX_HELPERS} prompt helpers. Delete one before adding another.`;

/** Moves `id` up (-1) or down (+1) past the neighbour that shares `kind` (Settings shows one
 *  asset type at a time; the order is global). Returns the list unchanged at an end. */
export function moveWithin<T extends { key: string; kinds: readonly ProjectKind[] }>(
  list: readonly T[],
  key: string,
  kind: ProjectKind,
  dir: -1 | 1,
): T[] {
  const at = list.findIndex((h) => h.key === key);
  if (at < 0) return [...list];
  let other = at + dir;
  while (other >= 0 && other < list.length && !list[other]!.kinds.includes(kind)) other += dir;
  if (other < 0 || other >= list.length) return [...list];
  const next = [...list];
  [next[at], next[other]] = [next[other]!, next[at]!];
  return next;
}
