// PRM thin e2e (prompt-helper §13, slice 6) on the mocked core: open the chat → consent → send
// → proposal → Replace → the main prompt field updates. The Chat button asks for the window
// (`prm_window_open`); the test then opens the chat page (`/?window=prompt-chat`) and relays
// its IPC and events to the main page's mock core through Playwright bindings, so both windows
// share one core, as the two webviews do in the app.
import { expect, test, type Page } from '@playwright/test';

import { calls } from './mock-core';
import { drawRegion, openRgn } from './rgn-mock';

/** The chat page's `__TAURI_INTERNALS__`: every call goes to the main page through bindings. */
function chatInternals(): void {
  type Rec = Record<string, unknown>;
  type Reply = { ok: boolean; value: unknown };
  const w = window as unknown as Rec & {
    __pfBridgeInvoke(cmd: string, args: Rec): Promise<Reply>;
    __pfBridgeListen(event: string, handler: number): Promise<number>;
  };
  const callbacks = new Map<number, (p: unknown) => void>();
  let next = 1;
  w.__pfCallbacks = callbacks;
  w.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: 'prompt-chat' },
      currentWebview: { windowLabel: 'prompt-chat', label: 'prompt-chat' },
    },
    transformCallback(cb: (p: unknown) => void) {
      const id = next++;
      callbacks.set(id, cb);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc: (p: string) => p,
    async invoke(cmd: string, args: Rec) {
      if (cmd === 'plugin:event|listen')
        return w.__pfBridgeListen(String(args.event), Number(args.handler));
      if (cmd === 'plugin:event|unlisten') return null;
      const r = await w.__pfBridgeInvoke(cmd, args);
      if (!r.ok) throw r.value;
      return r.value;
    },
  };
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
}

type Main = {
  __TAURI_INTERNALS__: {
    invoke(cmd: string, args: unknown): Promise<unknown>;
    transformCallback(cb: (p: unknown) => void): number;
  };
  __pfBridgeEmit(handler: number, payload: unknown): void;
};

async function openChatPage(main: Page): Promise<Page> {
  const chat = await main.context().newPage();
  await main.exposeFunction('__pfBridgeEmit', (handler: number, payload: unknown) =>
    chat.evaluate(
      ([h, p]) =>
        (
          window as unknown as { __pfCallbacks: Map<number, (p: unknown) => void> }
        ).__pfCallbacks.get(h as number)?.(p),
      [handler, payload] as const,
    ),
  );
  await chat.exposeFunction('__pfBridgeInvoke', (cmd: string, args: unknown) =>
    main.evaluate(
      ([c, a]) =>
        (window as unknown as Main).__TAURI_INTERNALS__.invoke(c as string, a).then(
          (value) => ({ ok: true, value }),
          (value: unknown) => ({ ok: false, value }),
        ),
      [cmd, args] as const,
    ),
  );
  await chat.exposeFunction('__pfBridgeListen', (event: string, handler: number) =>
    main.evaluate(
      ([e, h]) => {
        const w = window as unknown as Main;
        const id = w.__TAURI_INTERNALS__.transformCallback((p) => w.__pfBridgeEmit(h as number, p));
        return w.__TAURI_INTERNALS__.invoke('plugin:event|listen', { event: e, handler: id });
      },
      [event, handler] as const,
    ),
  );
  await chat.addInitScript(chatInternals);
  await chat.goto('/?window=prompt-chat');
  return chat;
}

test('FR-PRM-20 FR-PRM-21 FR-PRM-27 FR-PRM-29 open the chat → consent → send → proposal → Replace → the prompt field updates', async ({
  page,
}) => {
  await openRgn(page);
  await drawRegion(page, [400, 500], [800, 900]);
  await page.getByTestId('prm-block-prompt').fill('a room');

  await page.getByTestId('prm-block-chat').click();
  await expect
    .poll(async () => (await calls(page)).some((c) => c.cmd === 'prm_window_open'))
    .toBe(true);
  const chat = await openChatPage(page);
  await expect(chat.getByTestId('prm-chat-fake')).toHaveText('Fake chat');
  await chat.getByTestId('prm-chat-consent-accept').click();
  await expect(chat.getByTestId('prm-chat-host')).toHaveText('Region edit');

  await chat.getByTestId('prm-chat-input').fill('make this a kitchen');
  await chat.getByTestId('prm-chat-send').click();
  const card = chat.getByTestId('prm-chat-proposal');
  await expect(card).toHaveCount(1);
  await expect(card.locator('code')).toHaveText('make this a kitchen');
  await expect(chat.getByTestId('prm-chat-usage')).toContainText('≈ $');

  await chat.getByTestId('prm-chat-proposal-replace').click();
  await expect(page.getByTestId('prm-block-prompt')).toHaveValue('make this a kitchen');
  await expect(card).toContainText('Inserted');
});
