import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { api, appReady, currentWindowLabel } from './api';
import { ChatWindow } from './features/prompt-helper';
import { App } from './shell/App';
import './shell/theme.css';
import './shell/styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root');
// The same entry serves the prompt chat window (label `prompt-chat`, created from Rust, R91).
const chatWindow = currentWindowLabel() === 'prompt-chat';
createRoot(root).render(<StrictMode>{chatWindow ? <ChatWindow /> : <App />}</StrictMode>);

// Report once the shell rendered and the core answered (tauri:smoke waits for this). Only a
// debug smoke run answers with an image: then check one tile through the pf-tiles scheme (R46).
requestAnimationFrame(() => {
  if (chatWindow) return;
  api.appInfo().then(
    async (info) => {
      const image = await appReady(true, `core ${info.version}, provider ${info.provider_mode}`);
      if (!image) return;
      const { smokeTile } = await import('./smokeTile');
      const r = await smokeTile(image);
      await appReady(r.ok, r.detail);
    },
    (e: Error) => appReady(false, e.message),
  );
});
