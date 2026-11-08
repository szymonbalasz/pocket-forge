// "Open the settings" across features: the section rail's Settings entry (PRJ) asks, the
// provider feature answers with its settings dialog. Features never import each other's
// internals, so they meet here. Other features add their own sections to that dialog through
// `registerSettingsSection` (e.g. RGN's blend instruction, #55).
import type { ComponentType } from 'react';

type Listener = () => void;
const listeners = new Set<Listener>();

/** Asks whoever shows the settings to open them. */
export function requestSettings(): void {
  for (const l of listeners) l();
}

/** Subscribes to settings requests. Returns the unsubscribe function. */
export function onSettingsRequested(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** A section another feature adds to the settings dialog, below the provider's. */
export interface SettingsSection {
  /** Unique; the section wrapper gets `data-section="<id>"`. */
  id: string;
  title: string;
  Component: ComponentType;
}

const sections: SettingsSection[] = [];

/** Adds a section (idempotent per id; registration order is render order). */
export function registerSettingsSection(section: SettingsSection): void {
  if (!sections.some((s) => s.id === section.id)) sections.push(section);
}

export const settingsSections = (): readonly SettingsSection[] => sections;
