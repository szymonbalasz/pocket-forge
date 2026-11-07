// The window shell: the slot layout (Layout.tsx) filled from the feature registry, plus app-level
// banners and the section-rail placeholder until PRJ fills the rail (#44, #45).
import { useEffect, useState } from 'react';
import { Tooltip } from 'radix-ui';

import { api, type AppInfo } from '../api';
import { openProviderSettings } from '../features/provider';
import { features } from './features';
import { IconItems, IconMaps, IconSettings, IconTokens } from './icons';
import { Layout } from './Layout';

/** Placeholder section rail (PRJ replaces it by registering a `rail` component). */
function RailPlaceholder({ info }: { info: AppInfo | null }) {
  const items = [
    { id: 'maps', label: 'Maps', Icon: IconMaps },
    { id: 'tokens', label: 'Tokens', Icon: IconTokens },
    { id: 'items', label: 'Items', Icon: IconItems },
  ];
  return (
    <>
      <div className="pf-rail-brand" title="Pocket Forge">
        PF
      </div>
      {items.map(({ id, label, Icon }) => (
        <button
          key={id}
          type="button"
          className="pf-rail-item"
          aria-label={label}
          title={label}
          aria-current={id === 'maps' ? 'page' : undefined}
          data-testid={`app-rail-${id}`}
        >
          <Icon size={20} />
          <span>{label}</span>
        </button>
      ))}
      <span className="pf-spacer" />
      {info && (
        <span className="pf-chip" data-testid="prv-status-mode" title="Generation provider">
          {info.provider_mode === 'fake' ? 'Fake' : 'Scenario'}
        </span>
      )}
      <button
        type="button"
        className="pf-rail-item"
        aria-label="Settings"
        title="Settings"
        data-testid="app-rail-settings"
        onClick={openProviderSettings}
      >
        <IconSettings size={20} />
      </button>
    </>
  );
}

export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo, () => undefined);
  }, []);

  return (
    <Tooltip.Provider delayDuration={400}>
      <Layout
        features={features}
        railFallback={<RailPlaceholder info={info} />}
        banners={info?.warnings.map((w) => (
          <div key={w.code} className="banner banner-warning" data-testid="app-warning">
            {w.message}
          </div>
        ))}
      />
    </Tooltip.Provider>
  );
}
