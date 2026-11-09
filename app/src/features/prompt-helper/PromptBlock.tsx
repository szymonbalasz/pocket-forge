// The prompt block (prompt-helper §7, FR-PRM-04..12): the typed prompt with its placeholder and
// Tips, the helpers row (toggle chips), the host's own rows, "Final prompt (as sent)" with part
// markers and a character count, and the Chat button. Rendered by RGN (Region edit tab) and GEN
// (New image tab) through this feature's index.ts. It is controlled: the host owns the typed
// prompt and the ticks and saves them with its draft (FR-PRM-05); the preview comes from
// `prm_compose_preview`, never from a provider call (FR-PRM-08).
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Popover, Tooltip } from 'radix-ui';

import { api, onEvent, type ComposePreview, type ProjectKind, type PromptHelper } from '../../api';
import { chatContext } from '../../shared/chatContext';
import { projectStore } from '../../store/project';
import {
  PLACEHOLDER,
  TIPS,
  effectiveTicks,
  rowHelpers,
  skipLabel,
  skipTitle,
  toggleTick,
  type PromptHost,
} from './logic';
import { useHelpers } from './store';
import './prm.css';

/** FR-PRM-10: what a Retry restored (from the host's retry draft). */
export interface PromptRetry {
  /** The iteration `NNN` the job made (`changed since NNN`). */
  since: string;
  /** Ticked ids whose text changed since that job. */
  changed: readonly string[];
  /** Helpers the job sent that no longer exist (or are hidden / not for this kind). */
  missing: readonly { id: string; label: string }[];
}

export interface PromptBlockProps {
  host: PromptHost;
  /** The project's asset type (FR-PRM-04). */
  kind: ProjectKind;
  /** The typed prompt. */
  prompt: string;
  onPromptChange(text: string): void;
  /** The draft's ticks; null = the defaults (FR-PRM-05). */
  helperIds: readonly string[] | null;
  /** A tick change: always the explicit list, to save with the draft. */
  onHelperIdsChange(ids: string[]): void;
  /** RGN: the blend instruction as appended while it is on; null = none (GEN, or off). */
  blendInstruction?: string | null;
  /** Read-only project (FR-PRM-12): shown, not editable. */
  readOnly?: boolean;
  /** The model requires a prompt (shows the `*`). */
  required?: boolean;
  /** FR-PRM-10 badges after a Retry. */
  retry?: PromptRetry | null;
  /** The host's rows between the helpers row and the preview (RGN's blend toggle). */
  children?: ReactNode;
  /** Called with each preview (e.g. for the host's confirm dialog). */
  onPreview?(preview: ComposePreview): void;
}

/** FR-PRM-29 (slice 5): Replace from the chat reaches only the turn host's attached block, for
 *  the open project, and is applied as a normal edit (the host saves the draft; the preview
 *  follows). A read-only block ignores it. */
function useChatInsert(host: PromptHost, readOnly: boolean, apply: (text: string) => void): void {
  const latest = useRef(apply);
  latest.current = apply;
  useEffect(() => {
    if (readOnly) return;
    let off: (() => void) | null = null;
    let gone = false;
    void onEvent('prm:insert', (p) => {
      if (p.host === host && p.project_id === projectStore.get().project?.project_id)
        latest.current(p.text);
    }).then(
      (u) => {
        if (gone)
          void Promise.resolve()
            .then(u)
            .catch(() => undefined);
        else off = u;
      },
      () => undefined,
    );
    return () => {
      gone = true;
      const u = off;
      if (u)
        void Promise.resolve()
          .then(u)
          .catch(() => undefined);
    };
  }, [host, readOnly]);
}

export function PromptBlock(props: PromptBlockProps) {
  const { host, kind, prompt, helperIds, readOnly = false } = props;
  const blend = props.blendInstruction ?? null;
  const { helpers, error } = useHelpers();
  const preview = usePreview(kind, prompt, helperIds, blend, helpers, props.onPreview);
  useChatInsert(host, readOnly, props.onPromptChange);
  const id = `prm-prompt-${host}`;

  return (
    <div className="rgn-section prm-block" data-testid="prm-block" data-host={host}>
      <div className="prm-block-head">
        <label className="rgn-control-label" htmlFor={id}>
          Prompt
          {props.required && (
            <Tooltip.Provider delayDuration={400}>
              <Tooltip.Root>
                <Tooltip.Trigger asChild data-tooltip="Required">
                  <span className="rgn-required" aria-label="Required">
                    *
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Portal>
                  <Tooltip.Content className="pf-tooltip" side="top" sideOffset={6}>
                    Required
                  </Tooltip.Content>
                </Tooltip.Portal>
              </Tooltip.Root>
            </Tooltip.Provider>
          )}
        </label>
        <span className="pf-spacer" />
        <Tips />
      </div>
      <textarea
        id={id}
        className="rgn-prompt"
        data-testid="prm-block-prompt"
        rows={4}
        value={prompt}
        placeholder={PLACEHOLDER[host]}
        readOnly={readOnly}
        onChange={(e) => props.onPromptChange(e.target.value)}
      />
      {helpers && (
        <HelpersRow
          helpers={helpers}
          kind={kind}
          ticks={helperIds}
          readOnly={readOnly}
          skipped={preview?.skipped ?? []}
          retry={props.retry ?? null}
          onChange={props.onHelperIdsChange}
        />
      )}
      {error && (
        <p className="error small" data-testid="prm-block-error" data-code={error.code}>
          Prompt helpers unavailable: {error.message}
        </p>
      )}
      {props.children}
      <FinalPrompt
        preview={preview}
        helpers={helpers ?? []}
        awaitingPrompt={!!props.required && prompt.trim() === ''}
      />
      <div className="prm-block-actions">
        {/* FR-PRM-21: opens or focuses the chat window; the block's context is published
            first (FR-PRM-23). */}
        <button
          type="button"
          data-testid="prm-block-chat"
          aria-label="Chat about this prompt (opens the chat window)"
          onClick={() => {
            void chatContext.flush();
            void api.chat.openWindow().catch(() => undefined);
          }}
        >
          Chat
        </button>
      </div>
    </div>
  );
}

/** `prm_compose_preview` on every change; a late answer to an older input is dropped. */
function usePreview(
  kind: ProjectKind,
  prompt: string,
  ticks: readonly string[] | null,
  blend: string | null,
  helpers: PromptHelper[] | null,
  onPreview: ((p: ComposePreview) => void) | undefined,
): ComposePreview | null {
  const [preview, setPreview] = useState<ComposePreview | null>(null);
  const seq = useRef(0);
  const report = useRef(onPreview);
  report.current = onPreview;
  const ticksKey = ticks ? ticks.join('\n') : null;
  useEffect(() => {
    if (!helpers) return;
    const mine = ++seq.current;
    api.prompt
      .composePreview({
        kind,
        typed_prompt: prompt,
        helper_ids: ticksKey === null ? null : ticksKey === '' ? [] : ticksKey.split('\n'),
        blend_instruction: blend,
      })
      .then(
        (p) => {
          if (mine !== seq.current) return;
          setPreview(p);
          report.current?.(p);
        },
        () => {
          if (mine === seq.current) setPreview(null);
        },
      );
  }, [kind, prompt, ticksKey, blend, helpers]);
  return preview;
}

function Tips() {
  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button type="button" className="prm-tips-trigger small" data-testid="prm-block-tips">
          Tips
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          className="prm-tips"
          side="left"
          align="start"
          sideOffset={6}
          data-testid="prm-block-tips-content"
        >
          <ul>
            {TIPS.map((t) => (
              <li key={t}>{t}</li>
            ))}
          </ul>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

interface RowProps {
  helpers: PromptHelper[];
  kind: ProjectKind;
  ticks: readonly string[] | null;
  readOnly: boolean;
  skipped: ComposePreview['skipped'];
  retry: PromptRetry | null;
  onChange(ids: string[]): void;
}

/** FR-PRM-04/05/07/10/12: the visible helpers of this kind as toggle chips. */
function HelpersRow({ helpers, kind, ticks, readOnly, skipped, retry, onChange }: RowProps) {
  const row = rowHelpers(helpers, kind);
  const on = new Set(effectiveTicks(ticks, helpers, kind));
  if (row.length === 0 && !retry?.missing.length) return null;
  return (
    <div
      className="prm-helpers"
      data-testid="prm-block-helpers"
      aria-disabled={readOnly || undefined}
    >
      <Tooltip.Provider delayDuration={400}>
        {row.map((h) => {
          const skip = on.has(h.id) ? skipped.find((s) => s.id === h.id) : undefined;
          const changed = on.has(h.id) && retry?.changed.includes(h.id);
          return (
            <Tooltip.Root key={h.id}>
              <Tooltip.Trigger asChild>
                <button
                  type="button"
                  className="prm-chip"
                  data-testid="prm-block-helper"
                  data-record-id={h.id}
                  data-skipped={skip?.reason}
                  aria-pressed={on.has(h.id)}
                  disabled={readOnly}
                  onClick={() => onChange(toggleTick(ticks, helpers, kind, h.id))}
                >
                  {h.label}
                  {skip && (
                    <span className="prm-chip-badge" data-testid="prm-block-helper-skipped">
                      {skipLabel(skip.reason)}
                    </span>
                  )}
                  {changed && (
                    <span className="prm-chip-badge" data-testid="prm-block-helper-changed">
                      changed since {retry!.since}
                    </span>
                  )}
                </button>
              </Tooltip.Trigger>
              <Tooltip.Portal>
                <Tooltip.Content className="pf-tooltip prm-chip-tip" side="top" sideOffset={4}>
                  {h.text}
                  {/* L5 (R85): the skip reason is part of the chip's Radix tip. */}
                  {skip && <span className="prm-chip-tip-note">{skipTitle(skip.reason)}</span>}
                </Tooltip.Content>
              </Tooltip.Portal>
            </Tooltip.Root>
          );
        })}
      </Tooltip.Provider>
      {retry && retry.missing.length > 0 && (
        <p className="muted small prm-missing" data-testid="prm-block-missing">
          Missing since {retry.since}, not sent: {retry.missing.map((m) => m.label).join(', ')}
        </p>
      )}
    </div>
  );
}

/** What a blank required prompt would still add at send (BL-39 note). */
function addedNote(preview: ComposePreview | null): string | null {
  const has = (src: string) => !!preview?.parts.some((p) => p.source === src);
  const added = [has('helper') && 'helpers', has('blend') && 'blend instruction'].filter(Boolean);
  return added.length ? `+ ${added.join(' and ')} at send` : null;
}

/** FR-PRM-08: the composed text exactly as sent, each part marked by its source (typed, the
 *  helper's label, blend instruction), and its character count. BL-39: while a required prompt
 *  is blank (submit is blocked) it shows an empty prompt and a note, not the added parts alone. */
function FinalPrompt({
  preview,
  helpers,
  awaitingPrompt,
}: {
  preview: ComposePreview | null;
  helpers: PromptHelper[];
  awaitingPrompt: boolean;
}) {
  const note = awaitingPrompt ? addedNote(preview) : null;
  const source = (p: ComposePreview['parts'][number]) =>
    p.source === 'typed'
      ? 'Typed prompt'
      : p.source === 'blend'
        ? 'Blend instruction'
        : (helpers.find((h) => h.id === p.id)?.label ?? 'Helper');
  return (
    <details className="rgn-final prm-final" data-testid="prm-block-final">
      <summary className="muted small">
        Final prompt (as sent){' '}
        <span className="tabular" data-testid="prm-block-chars">
          {preview && !awaitingPrompt ? `· ${preview.chars} characters` : ''}
        </span>
      </summary>
      {preview && preview.parts.length > 0 && !awaitingPrompt ? (
        <Tooltip.Provider delayDuration={400}>
          <pre className="small prm-final-text" data-testid="prm-block-final-prompt">
            {preview.parts.map((p, i) => {
              const prev = preview.parts[i - 1];
              // The core joins helper texts with one space and the three blocks with a blank line.
              const sep = !prev
                ? ''
                : prev.source === 'helper' && p.source === 'helper'
                  ? ' '
                  : '\n\n';
              return (
                <span key={`${p.source}-${p.id ?? i}`}>
                  {sep}
                  {/* L5 (R85): each part's source as a Radix tip. */}
                  <Tooltip.Root>
                    <Tooltip.Trigger asChild data-tooltip={source(p)}>
                      <span
                        className="prm-part"
                        data-testid="prm-block-part"
                        data-source={p.source}
                        data-record-id={p.id}
                      >
                        {p.text}
                      </span>
                    </Tooltip.Trigger>
                    <Tooltip.Portal>
                      <Tooltip.Content className="pf-tooltip" side="top" sideOffset={4}>
                        {source(p)}
                      </Tooltip.Content>
                    </Tooltip.Portal>
                  </Tooltip.Root>
                </span>
              );
            })}
          </pre>
        </Tooltip.Provider>
      ) : (
        <>
          <pre className="small muted" data-testid="prm-block-final-prompt" data-empty="true">
            —
          </pre>
          {note && (
            <p className="muted small" data-testid="prm-block-final-note">
              {note}
            </p>
          )}
        </>
      )}
    </details>
  );
}
