import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { api, appReady } from './api';
import { App } from './shell/App';
import './shell/styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('missing #root');
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Report once the shell rendered and the core answered (tauri:smoke waits for this).
requestAnimationFrame(() => {
  api.appInfo().then(
    (info) => appReady(true, `core ${info.version}, provider ${info.provider_mode}`),
    (e: Error) => appReady(false, e.message),
  );
});
