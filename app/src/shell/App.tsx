// The window shell: a library side panel, the editor area and a status bar. Each region renders
// the registered features for its slot (shell/features.ts).
import { useEffect, useState } from 'react';

import { api, type AppInfo } from '../api';
import { features } from './features';
import type { Slot } from '../shared/feature';

function SlotView({ slot }: { slot: Slot }) {
  return (
    <>
      {features
        .filter((f) => f.slot === slot)
        .map((f) => (
          <f.Component key={f.id} />
        ))}
    </>
  );
}

export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo, () => undefined);
  }, []);

  return (
    <div className="shell" data-testid="app-shell">
      <aside className="sidebar">
        <header className="brand">
          Pocket Forge
          {info && (
            <span className="chip" data-testid="prv-status-mode" title="Generation provider">
              {info.provider_mode === 'fake' ? 'Fake provider' : 'Scenario'}
            </span>
          )}
        </header>
        {info?.warnings.map((w) => (
          <div key={w.code} className="banner banner-warning" data-testid="app-warning">
            {w.message}
          </div>
        ))}
        <SlotView slot="sidebar" />
      </aside>
      <main className="main">
        <SlotView slot="toolbar" />
        <SlotView slot="main" />
      </main>
      <footer className="statusbar">
        <SlotView slot="status" />
      </footer>
    </div>
  );
}
