import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { api, appReady } from './api';
import { App } from './shell/App';
import './shell/theme.css';
import './shell/styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root');
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Report once the shell rendered and the core answered (tauri:smoke waits for this). Only a
// debug smoke run answers with an image: then check one tile through the pf-tiles scheme (R46).
requestAnimationFrame(() => {
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
