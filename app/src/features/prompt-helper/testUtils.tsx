// Vitest harness for the prompt block (test-only; never imported by app code): the fake core
// with the PRM mock, jsdom polyfills Radix needs, and a host stand-in that owns the typed prompt
// and the ticks the way RGN and GEN will (slice 3, GEN slice 6).
import { useState } from 'react';

import type { ComposePreview, PromptHelper } from '../../api';
import { installPrmMock, type PrmMockSeed } from '../../api/mocks/prm';
import { installFakeTauri, type FakeTauri } from '../../api/mocks/tauri';
import { PromptBlock, type PromptBlockProps } from './PromptBlock';
import { resetHelpersStore } from './store';

export function polyfillRadix(): void {
  const g = globalThis as unknown as Record<string, unknown>;
  g.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  const proto = Element.prototype as unknown as Record<string, unknown>;
  proto.hasPointerCapture ??= () => false;
  proto.setPointerCapture ??= () => undefined;
  proto.releasePointerCapture ??= () => undefined;
  proto.scrollIntoView ??= () => undefined;
}

/** The fake core with only the PRM commands (plus the fake's built-ins). */
export function mockPrm(seed: PrmMockSeed = {}): FakeTauri {
  polyfillRadix();
  resetHelpersStore();
  const fake = installFakeTauri();
  installPrmMock(seed);
  return fake;
}

/** A user-made helper for seeds. */
export function userHelper(id: string, patch: Partial<PromptHelper> = {}): PromptHelper {
  return {
    id,
    label: 'Mine',
    text: 'Mine.',
    kinds: ['map'],
    default_on: false,
    builtin: false,
    hidden: false,
    modified: false,
    ...patch,
  };
}

export interface HostLog {
  ticks: string[][];
  previews: ComposePreview[];
}

/** A host that keeps the prompt and the ticks in state, as RGN/GEN keep them in their drafts. */
export function TestHost(
  props: Partial<Omit<PromptBlockProps, 'helperIds'>> & {
    initialTicks?: string[] | null;
    log?: HostLog;
  },
) {
  const { initialTicks = null, log, ...rest } = props;
  const [prompt, setPrompt] = useState(props.prompt ?? '');
  const [ticks, setTicks] = useState<string[] | null>(initialTicks);
  return (
    <PromptBlock
      host="rgn"
      kind="map"
      {...rest}
      prompt={prompt}
      onPromptChange={setPrompt}
      helperIds={ticks}
      onHelperIdsChange={(ids) => {
        log?.ticks.push(ids);
        setTicks(ids);
      }}
      onPreview={(p) => log?.previews.push(p)}
    />
  );
}
