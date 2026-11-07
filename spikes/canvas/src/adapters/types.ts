import type { Pt, View } from '../core/view';
import type { TileSink } from '../core/tiles';
import type { Surface } from '../core/mask';

export type LibName = 'konva' | 'fabric' | 'pixi';

/** Patch layer transform, in image px. Rotation about the patch centre, degrees. */
export interface PatchProps { cx: number; cy: number; scale: number; rotation: number; opacity: number; visible: boolean }

/** What each library must provide. Everything else (tiles, mask, view math) is shared code. */
export interface Adapter extends TileSink {
  readonly name: LibName;
  init(el: HTMLElement, w: number, h: number): Promise<void>;
  setView(v: View): void;
  /** Synchronous render (for CPU timing). */
  render(): void;
  /** Library-native coordinate conversion (the accuracy test checks these against core/view). */
  screenToImage(p: Pt): Pt;
  imageToScreen(p: Pt): Pt;
  addMask(surfaces: Surface[], opacity: number): void;
  updateMask(dirty: Surface[]): void;
  addPatch(img: CanvasImageSource & { width: number; height: number }, p: PatchProps): void;
  setPatch(p: Partial<PatchProps>): void;
  /** Patch corners in image px, read back from the library's own transform (tl, tr, br, bl). */
  patchCorners(): Pt[];
  /** Turn on interactive move/scale/rotate handles; returns what the library gave for free. */
  enableHandles(on: boolean): { move: boolean; scale: boolean; rotate: boolean; builtIn: boolean };
  /** Let the library receive pointer events (layer mode) or not (pan/mask mode). */
  setInteractive(on: boolean): void;
  info(): Record<string, unknown>;
  destroy(): void;
}
