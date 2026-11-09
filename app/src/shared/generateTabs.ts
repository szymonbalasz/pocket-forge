// The Generate area's tab strip (ui-guidelines § Layout, OQ-GEN-09): **Region edit** (RGN) |
// **New image** (GEN). RGN's Generate section draws the strip and its own tab; other features add
// tabs here, so no feature renders another's internals. Only the visible tab is mounted
// (FR-GEN-36). A project with no image shows only the tabs that work without one.
import { useSyncExternalStore, type ComponentType } from 'react';

/** RGN's own tab. */
export const REGION_TAB = 'region';
/** GEN's New image tab (FR-GEN-04); RGN's tracker routes generate jobs' Retry to it. */
export const NEW_IMAGE_TAB = 'new-image';

export interface GenerateTab {
  /** Unique; the tab button gets `data-record-id="<id>"`. */
  id: string;
  label: string;
  Component: ComponentType;
  /** Also offered on a project with no image yet (FR-GEN-03/04). */
  withoutImage: boolean;
  /** Retry of one of this tab's jobs from RGN's tracker (FR-GEN-32): the tab is shown first. */
  retry?(jobId: string): void;
}

const tabs: GenerateTab[] = [];

/** Adds a tab after Region edit (idempotent per id; registration order is strip order). */
export function registerGenerateTab(tab: GenerateTab): void {
  if (!tabs.some((t) => t.id === tab.id)) tabs.push(tab);
}

export const generateTabs = (): readonly GenerateTab[] => tabs;

/** A project with no image (FR-GEN-02/03): `original: null` → no `image_path`. */
export const projectHasImage = (p: { image_path: string | null }): boolean => !!p.image_path;

let selected = REGION_TAB;
/** The tab the next opened project starts on (+ New → Generate new…, FR-GEN-01). */
let nextOpen: string | null = null;
/** The open project, whose shown tab is remembered (R85, R94). */
let openProject: string | null = null;

/** R85 / R94: the shown tab per project, per machine (this webview's storage), so it persists
 *  across closing and reopening a project and across a restart. A UI preference only: storage
 *  that fails just forgets. */
const STORE_KEY = 'pf.generate-tab';
const STORE_MAX = 200;
function rememberedTabs(): Record<string, string> {
  try {
    const v: unknown = JSON.parse(globalThis.localStorage?.getItem(STORE_KEY) ?? '{}');
    return v && typeof v === 'object' ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}
function remember(projectId: string, tab: string): void {
  try {
    const all = rememberedTabs();
    delete all[projectId];
    all[projectId] = tab;
    const keys = Object.keys(all);
    for (const k of keys.slice(0, Math.max(0, keys.length - STORE_MAX))) delete all[k];
    globalThis.localStorage?.setItem(STORE_KEY, JSON.stringify(all));
  } catch {
    // No storage: the tab just isn't remembered.
  }
}
const listeners = new Set<() => void>();
const emit = () => {
  for (const l of listeners) l();
};

export const generateTab = {
  get: (): string => selected,
  select(id: string): void {
    if (openProject && rememberedTabs()[openProject] !== id) remember(openProject, id);
    if (id === selected) return;
    selected = id;
    emit();
  },
  subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  /** The next project that opens starts on this tab. */
  openNextWith(id: string): void {
    nextOpen = id;
  },
  /** A project was opened: its starting tab (the requested one, else the one last shown for
   *  this project, R85 / R94, else Region edit). */
  projectOpened(projectId?: string): void {
    openProject = projectId ?? null;
    const id = nextOpen ?? (projectId ? rememberedTabs()[projectId] : undefined) ?? REGION_TAB;
    nextOpen = null;
    generateTab.select(id);
  },
  /** Tests: forget every remembered tab. */
  forgetAll(): void {
    openProject = null;
    try {
      globalThis.localStorage?.removeItem(STORE_KEY);
    } catch {
      // No storage.
    }
  },
  /** Routes a tracker Retry to the tab that owns the job's operation; false = none does. */
  retry(tabId: string, jobId: string): boolean {
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab?.retry) return false;
    generateTab.select(tabId);
    tab.retry(jobId);
    return true;
  },
};

export function useGenerateTab(): string {
  return useSyncExternalStore(generateTab.subscribe, generateTab.get, generateTab.get);
}
