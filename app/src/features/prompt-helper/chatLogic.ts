// Pure helpers for the chat panel (prompt-helper §7).
import type {
  ChatBackendStatus,
  ChatHostId,
  ChatStartedAfter,
  ChatStatus,
  ChatThread,
  ChatTurn,
} from '../../api';

/** #61: the one-line divider at the top of a thread that replaced another: why it started
 *  (FR-PRM-47, "New chat after accepting r12"), or "New chat" after the button; none for the
 *  project's first thread. */
const DIVIDER_VERB: Record<ChatStartedAfter['reason'], string> = {
  accepted: 'accepting',
  picked: 'picking',
  dropped: 'dropping in',
};

export function dividerText(t: ChatThread): string | null {
  const a = t.started_after;
  if (a) return `New chat after ${DIVIDER_VERB[a.reason]} r${a.seq}`;
  return t.previous_thread_id ? 'New chat' : null;
}

export const HOST_LABEL: Record<ChatHostId, string> = {
  rgn: 'Region edit',
  gen: 'New image',
};

/** One piece of a reply: plain text, or a fenced block shown as code (§7). */
export type Segment = { kind: 'text'; text: string } | { kind: 'code'; lang: string; text: string };

/** Splits a reply at ``` fences. An unclosed fence runs to the end. */
export function segments(text: string): Segment[] {
  const out: Segment[] = [];
  const lines = text.split('\n');
  let buf: string[] = [];
  let fence: { lang: string; body: string[] } | null = null;
  const flush = () => {
    const t = buf.join('\n');
    if (t.trim()) out.push({ kind: 'text', text: t.replace(/^\n+|\n+$/g, '') });
    buf = [];
  };
  for (const line of lines) {
    const m = /^\s*```(.*)$/.exec(line);
    if (fence) {
      if (m) {
        out.push({ kind: 'code', lang: fence.lang, text: fence.body.join('\n') });
        fence = null;
      } else fence.body.push(line);
    } else if (m) {
      flush();
      fence = { lang: m[1]!.trim(), body: [] };
    } else buf.push(line);
  }
  if (fence) out.push({ kind: 'code', lang: fence.lang, text: fence.body.join('\n') });
  flush();
  return out;
}

/** FR-PRM-38: the thread total for the footer, or null before any usage. On the fake backend
 *  the cost is notional and says so (L4). */
export function usageLabel(turns: readonly ChatTurn[], fake = false): string | null {
  let cost = 0;
  let any = false;
  let billed = false;
  for (const t of turns) {
    if (!t.usage) continue;
    any = true;
    cost += t.usage.cost_usd ?? 0;
    if (!t.usage.notional) billed = true;
  }
  if (!any) return null;
  if (fake) return `≈ $${cost.toFixed(2)} notional (fake chat, nothing billed)`;
  return `≈ $${cost.toFixed(2)} ${billed ? 'billed' : 'plan usage'}`;
}

const cap = (s: string) => (s ? s[0]!.toUpperCase() + s.slice(1) : s);

/** Short error labels for the window's header chip; the body shows the full message (L6). */
const SHORT_ERROR: Record<string, string> = {
  not_installed: 'Not installed',
  version_unsupported: 'Version too old',
  isolation_unavailable: 'Isolation unavailable',
  not_logged_in: 'Not logged in',
  usage_limit: 'Usage limit',
  offline: 'Offline',
  timeout: 'Timed out',
  backend_crashed: 'Crashed',
  stream_malformed: 'Backend error',
  backend_error: 'Backend error',
};

/** The backend chip: `Claude Code 2.1.319 · Max`, or the error message (`short`: a short label,
 *  for the window header, whose body already shows the message, L6). */
export function backendChip(status: ChatStatus, short = false): string {
  const b: ChatBackendStatus = status.backend;
  if (b.state === 'ok') {
    if (status.mode === 'fake') return 'Fake backend';
    return `Claude Code ${b.version}${b.plan ? ` · ${cap(b.plan)}` : ''}`;
  }
  if (b.state === 'error') return short ? (SHORT_ERROR[b.code] ?? 'Error') : b.message;
  return 'Not checked';
}

/** L6: the reply's segments to show. Once a reply's prompt blocks became proposal cards, those
 *  fences aren't shown again in the text (the first `proposals` non-empty `prompt` blocks). */
export function replySegments(text: string, proposals: number): Segment[] {
  let left = proposals;
  return segments(text).filter((s) => {
    if (left > 0 && s.kind === 'code' && s.lang.toLowerCase() === 'prompt' && s.text.trim()) {
      left -= 1;
      return false;
    }
    return true;
  });
}

/** L6: a fix that only says "Retry." is left out next to the Retry button. */
export const fixWorthShowing = (fix: string | undefined): fix is string =>
  !!fix && fix.trim().toLowerCase() !== 'retry.';

/** FR-PRM-45 (L2): what Log in… did. The fake opens no console and is logged in at once. */
export function loginNote(status: ChatStatus): string {
  return status.mode === 'fake'
    ? 'Fake backend: logged in without a console. Recheck to continue.'
    : 'A console opened: log in there, then Recheck.';
}

/** FR-PRM-38: the strip warns once per thread when the chat bills an API key. */
export function billsApiKey(status: ChatStatus): boolean {
  return status.backend.state === 'ok' && status.backend.auth_method === 'api_key';
}

/** The user turn a reply answers (the one just before it), for Retry. */
export function questionOf(turns: readonly ChatTurn[], replyId: string): ChatTurn | null {
  const i = turns.findIndex((t) => t.id === replyId);
  for (let j = i - 1; j >= 0; j--) if (turns[j]!.role === 'user') return turns[j]!;
  return null;
}

/** Appendix C, version 1. */
export const PRIVACY_NOTICE = [
  'Prompt chat sends data to Anthropic.',
  "When you send a message, Pocket Forge passes it to Claude Code on this computer. Claude Code sends it to Anthropic under the Claude login you set up for the chat, together with the image of the region you're editing (or your structural sketch and references), your prompt, the model and its settings. It counts toward your plan's usage, or is billed to your API key if one is set. Claude Code runs in its own folder inside Pocket Forge's data, without your usual Claude Code settings, and is told not to keep the conversation on disk. Anthropic's own data handling applies to what is sent. Nothing is sent until you press Send, and nothing the chat says reaches Scenario unless you insert it and press Generate yourself.",
] as const;
