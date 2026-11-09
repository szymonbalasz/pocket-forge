// The prompt chat's live context seam (prompt-helper FR-PRM-23 / FR-PRM-43, new-asset-generation
// FR-GEN-36): the visible tab's prompt block publishes its draft input (the same input its
// estimate takes), debounced 300 ms, and clears it when it is hidden or unmounted; the next
// chat turn's host is the tab that published last. PRM's chat core (slice 5) installs the sink
// that forwards to `prm_context_publish` / `prm_context_clear`, and calls `flush()` right before
// a send. Without a sink the input is only held here. Hosts never import each other.

/** A prompt block host: Region edit or New image (FR-PRM-43). */
export type ChatHostId = 'rgn' | 'gen';

/** Where published input goes (PRM slice 5: the core commands). */
export interface ChatContextSink {
  publish(host: ChatHostId, input: unknown): unknown;
  clear(host: ChatHostId): unknown;
}

/** FR-PRM-23: the publish debounce. */
export const CHAT_PUBLISH_MS = 300;

let sink: ChatContextSink | null = null;
let current: { host: ChatHostId; input: unknown } | null = null;
let pending: { host: ChatHostId; input: unknown; timer: ReturnType<typeof setTimeout> } | null =
  null;

function send(host: ChatHostId, input: unknown): Promise<void> {
  current = { host, input };
  return Promise.resolve(sink?.publish(host, input)).then(
    () => undefined,
    () => undefined,
  );
}

export const chatContext = {
  /** Schedules `input` as `host`'s context (a newer call replaces a pending one). */
  publish(host: ChatHostId, input: unknown): void {
    if (pending) clearTimeout(pending.timer);
    const timer = setTimeout(() => {
      pending = null;
      void send(host, input);
    }, CHAT_PUBLISH_MS);
    pending = { host, input, timer };
  },

  /** Publishes a pending input now (right before a chat send, FR-PRM-23). Resolves once the
   *  sink has it (at once when nothing is pending); never rejects. */
  flush(): Promise<void> {
    if (!pending) return Promise.resolve();
    clearTimeout(pending.timer);
    const { host, input } = pending;
    pending = null;
    return send(host, input);
  },

  /** `host`'s block was hidden or unmounted: its context goes (FR-GEN-36). */
  clear(host: ChatHostId): void {
    if (pending?.host === host) {
      clearTimeout(pending.timer);
      pending = null;
    }
    if (current?.host === host) current = null;
    void sink?.clear(host);
  },

  /** The published context the next turn is built from (its host is the turn's host). */
  current(): { host: ChatHostId; input: unknown } | null {
    return current;
  },

  /** PRM slice 5 installs the core sink (null removes it); the current input is sent at once. */
  setSink(next: ChatContextSink | null): void {
    sink = next;
    if (next && current) void next.publish(current.host, current.input);
  },

  /** Test helper. */
  reset(): void {
    if (pending) clearTimeout(pending.timer);
    pending = null;
    current = null;
    sink = null;
  },
};
