// The provider status chip (a status-bar segment, provider spec §7). Clicking it opens the
// provider settings, which also open from the rail's Settings entry (`shared/settings.ts`).
import { useEffect } from 'react';

import { chipOf } from './chip';
import { ProviderSettings } from './ProviderSettings';
import { providerStore, useProviderState } from './store';
import './provider.css';

export function ProviderChip() {
  const { status, balance } = useProviderState();

  useEffect(() => {
    void providerStore.refresh();
    void providerStore.listen();
  }, []);

  const chip = status ? chipOf(status, balance) : null;
  return (
    <>
      {chip && (
        <button
          type="button"
          className={`prv-chip prv-chip-${chip.state}`}
          data-testid="prv-status-chip"
          data-state={chip.state}
          title={chip.linksToSettings ? 'Open the provider settings' : 'Generation provider'}
          onClick={() => providerStore.openSettings()}
        >
          {chip.label}
        </button>
      )}
      <ProviderSettings />
    </>
  );
}
