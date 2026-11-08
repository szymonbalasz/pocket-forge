// The window shell: the slot layout (Layout.tsx) filled from the feature registry, plus
// app-level banners. PRJ fills the section rail (#44, #45); the provider chip is a status-bar
// segment (R40).
import { useEffect, useState } from 'react';
import { Tooltip } from 'radix-ui';

import { api, type AppInfo } from '../api';
import { features } from './features';
import { Layout } from './Layout';

export function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  useEffect(() => {
    api.appInfo().then(setInfo, () => undefined);
  }, []);

  return (
    <Tooltip.Provider delayDuration={400}>
      <Layout
        features={features}
        banners={info?.warnings.map((w) => (
          <div key={w.code} className="banner banner-warning" data-testid="app-warning">
            {w.message}
          </div>
        ))}
      />
    </Tooltip.Provider>
  );
}
