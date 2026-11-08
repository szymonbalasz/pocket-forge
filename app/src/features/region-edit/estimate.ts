// FR-RGN-12: an estimate is requested 500 ms after the last change to a cost-affecting input.
// The caller passes the cost signature (`costSignature`) on every change; unchanged signatures
// (the prompt, other controls) never re-estimate.
export const ESTIMATE_DEBOUNCE_MS = 500;

export class EstimateDebouncer {
  private last: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly run: (signature: string) => void,
    private readonly delayMs = ESTIMATE_DEBOUNCE_MS,
  ) {}

  /** A new signature (null = no estimate possible) restarts the 500 ms wait. */
  update(signature: string | null): void {
    if (signature === this.last) return;
    this.last = signature;
    this.clear();
    if (signature === null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.run(signature);
    }, this.delayMs);
  }

  /** Estimates again for the current signature (after `estimate_stale`, a retry, …). */
  force(): void {
    const sig = this.last;
    this.last = null;
    this.update(sig);
  }

  get pending(): boolean {
    return this.timer !== null;
  }

  get signature(): string | null {
    return this.last;
  }

  clear(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  reset(): void {
    this.clear();
    this.last = null;
  }
}
