// Pure helpers for the chat window (prompt-helper §7, PRM slice 6).
import type { ChatBackendStatus, ChatHostId, ChatStatus, ChatTurn } from '../../api';

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

/** FR-PRM-38: the thread total for the footer, or null before any usage. */
export function usageLabel(turns: readonly ChatTurn[]): string | null {
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
  return `≈ $${cost.toFixed(2)} ${billed ? 'billed' : 'plan usage'}`;
}

const cap = (s: string) => (s ? s[0]!.toUpperCase() + s.slice(1) : s);

/** The header's backend chip: `Claude Code 2.1.319 · Max`, or the error message. */
export function backendChip(status: ChatStatus): string {
  const b: ChatBackendStatus = status.backend;
  if (b.state === 'ok') {
    if (status.mode === 'fake') return 'Fake backend';
    return `Claude Code ${b.version}${b.plan ? ` · ${cap(b.plan)}` : ''}`;
  }
  if (b.state === 'error') return b.message;
  return 'Not checked';
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
