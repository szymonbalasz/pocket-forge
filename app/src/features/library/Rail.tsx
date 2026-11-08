// The section rail (asset-project §7, ui-guidelines): Maps, Tokens, Items, References, then
// Settings at the bottom. Picking a section shows its home grid (and closes an open project);
// References shows REF's packs home instead (FR-PRJ-52).
import { homeView, useHomeView } from '../../shared/home';
import { requestSettings } from '../../shared/settings';
import { IconItems, IconMaps, IconReferences, IconSettings, IconTokens } from '../../shell/icons';
import { projectStore, useProjectState } from '../../store/project';
import { SECTIONS } from './format';
import { libraryStore, useLibraryState } from './store';

const ICONS = { maps: IconMaps, tokens: IconTokens, items: IconItems } as const;

export function Rail() {
  const { section } = useLibraryState();
  const { project } = useProjectState();
  const view = useHomeView();
  const refs = view === 'references' && !project;
  return (
    <>
      <div className="pf-rail-brand" title="Pocket Forge">
        PF
      </div>
      {SECTIONS.map(({ id, label }) => {
        const Icon = ICONS[id];
        const current = id === section && !refs;
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
            onClick={() => {
              homeView.set('projects');
              void libraryStore.setSection(id);
            }}
          >
            <Icon size={20} />
            <span>{label}</span>
          </button>
        );
      })}
      <button
        type="button"
        className="pf-rail-item"
        aria-label="References"
        title="References"
        aria-current={refs ? 'page' : undefined}
        data-testid="prj-rail-references"
        onClick={() => void showReferences()}
      >
        <IconReferences size={20} />
        <span>Refs</span>
      </button>
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

/** FR-PRJ-52: closes an open project (unless refused), then shows the packs home. */
async function showReferences(): Promise<void> {
  if (projectStore.get().project) {
    await projectStore.close();
    if (projectStore.get().project) return; // close refused (`unsaved`)
  }
  homeView.set('references');
}
