// The provider status chip (status bar slot, provider spec §7). It opens the provider settings,
// which also open from the rail's Settings entry (`openProviderSettings`).
import { useEffect } from 'react';

import { chipOf } from './chip';
import { ProviderSettings } from './ProviderSettings';
import { providerStore, useProviderState } from './store';
import './provider.css';

export function ProviderChip() {
  const { status, balance, settingsOpen } = useProviderState();

  useEffect(() => {
    void providerStore.refresh();
    void providerStore.listen();
  }, []);

  if (!status) return null;
  const chip = chipOf(status, balance);
  return (
    <>
      <button
        type="button"
        className={`prv-chip prv-chip-${chip.state}`}
        data-testid="prv-header-chip"
        data-state={chip.state}
        title={chip.linksToSettings ? 'Open the provider settings' : 'Generation provider'}
        onClick={() => providerStore.openSettings()}
      >
        {chip.label}
      </button>
      {settingsOpen && <ProviderSettings />}
    </>
  );
}
