// EDT slice 5 (FR-EDT-18, 28, 29): mask hand-off to PRJ's store API (`put_mask`) through
// `edt_mask_submit`. **Typed stub**: it waits for the asset-project store API on main. The live mask
// (`MaskController.buffer`, `version`) already provides the bytes and the change counter the
// debounced submit will use.
import type { MaskRef } from '../../api';
import type { MaskBuffer } from './mask';

export type MaskStatus =
  | { kind: 'saved' }
  | { kind: 'saving' }
  | { kind: 'unsaved'; reason: 'pending' | 'read_only' | 'locked' | 'conflict' | 'io' | 'error' };

/** Debounce after the last stroke end before submitting (R7, ≥ 2 s). */
export const MASK_SUBMIT_DEBOUNCE_MS = 2000;

export interface MaskHandoff {
  /** Submits the mask's w×h bytes for its inner rect; resolves to the stored mask's ref. */
  submit(projectId: string, mask: MaskBuffer): Promise<MaskRef>;
}

/** Placeholder until `edt_mask_submit` + PRJ `put_mask` exist: always rejects, never writes. */
export const pendingHandoff: MaskHandoff = {
  submit: () =>
    Promise.reject(
      Object.assign(new Error('mask hand-off is not built yet (EDT slice 5)'), {
        code: 'not_implemented',
      }),
    ),
};
