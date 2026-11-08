// The section rail (asset-project §7, ui-guidelines): Maps, Tokens, Items, then Settings at the
// bottom. Picking a section shows its home grid (and closes an open project).
import { requestSettings } from '../../shared/settings';
import { IconItems, IconMaps, IconSettings, IconTokens } from '../../shell/icons';
import { useProjectState } from '../../store/project';
import { SECTIONS } from './format';
import { libraryStore, useLibraryState } from './store';

const ICONS = { maps: IconMaps, tokens: IconTokens, items: IconItems } as const;

export function Rail() {
  const { section } = useLibraryState();
  const { project } = useProjectState();
  return (
    <>
      <div className="pf-rail-brand" title="Pocket Forge">
        PF
      </div>
      {SECTIONS.map(({ id, label }) => {
        const Icon = ICONS[id];
        const current = id === section;
        return (
          <button
            key={id}
            type="button"
            className="pf-rail-item"
            aria-label={label}
            title={label}
            // The section stays marked while one of its projects is open.
            aria-current={current ? (project ? 'true' : 'page') : undefined}
            data-testid={`prj-rail-${id}`}
            onClick={() => void libraryStore.setSection(id)}
          >
            <Icon size={20} />
            <span>{label}</span>
          </button>
        );
      })}
      <span className="pf-spacer" />
      <button
        type="button"
        className="pf-rail-item"
        aria-label="Settings"
        title="Settings"
        data-testid="prj-rail-settings"
        onClick={requestSettings}
      >
        <IconSettings size={20} />
      </button>
    </>
  );
}
