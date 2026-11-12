// The one context menu component (#63, EXP §7): project cards, the left-panel name, images in
// the project panel, Results, the canvas, picker items and pack images all build their menus from
// it. Presentational only: the owner passes the entries and their callbacks (shared imports
// nothing outside shared). Test ids: the menu `<testId>`, items `<itemPrefix>-<entry id>`
// (`itemPrefix` defaults to the menu's test id).
import type { ReactElement, ReactNode } from 'react';
import { ContextMenu as Menu } from 'radix-ui';

export interface MenuItem {
  kind?: 'item';
  /** Test id suffix: `<menu test id>-<id>`. */
  id: string;
  label: ReactNode;
  onSelect: () => void;
  /** Disabled, with the reason as its tooltip (`data-reason`). */
  disabled?: string | null;
  /** A keyboard hint at the right (`F2`). */
  hint?: string;
}

export interface MenuSub {
  kind: 'sub';
  id: string;
  label: ReactNode;
  disabled?: string | null;
  items: MenuItem[];
}

export type MenuEntry = MenuItem | MenuSub | { kind: 'separator' };

/** The image formats Export as offers, in menu order (EXP §7). */
export type MenuFormat = 'png' | 'jpeg' | 'webp';
const FORMAT_LABEL: Record<MenuFormat, string> = { png: 'PNG', jpeg: 'JPEG', webp: 'WebP' };

export interface ImageMenuActions {
  /** Save as… (absent: not offered). */
  onSaveAs?: () => void;
  /** Export as ▸ PNG / JPEG / WebP (absent: not offered, e.g. a pack image). */
  onExport?: (format: MenuFormat) => void;
  /** Why the whole submenu is disabled (`no_image`, `read_only`), or null. */
  exportDisabled?: string | null;
  /** Why one format is disabled (FR-EXP-11), or null. */
  formatDisabled?: Partial<Record<MenuFormat, string | null>>;
  /** Why Save as is disabled, or null. */
  saveAsDisabled?: string | null;
  /** Open in Explorer (absent: not offered). */
  onReveal?: () => void;
}

/** The image menu's entries (EXP §7): Save as…, Export as ▸, Open in Explorer, each only when
 *  its action is given. Test ids `<menu>-save-as`, `<menu>-export-{png,jpeg,webp}`,
 *  `<menu>-reveal` (the submenu trigger is `<menu>-export`). */
export function imageMenuEntries(a: ImageMenuActions): MenuEntry[] {
  const out: MenuEntry[] = [];
  if (a.onSaveAs) {
    out.push({
      id: 'save-as',
      label: 'Save as…',
      onSelect: a.onSaveAs,
      disabled: a.saveAsDisabled ?? null,
    });
  }
  const onExport = a.onExport;
  if (onExport) {
    out.push({
      kind: 'sub',
      id: 'export',
      label: 'Export as',
      disabled: a.exportDisabled ?? null,
      items: (['png', 'jpeg', 'webp'] as const).map((f) => ({
        id: `export-${f}`,
        label: FORMAT_LABEL[f],
        onSelect: () => onExport(f),
        disabled: a.formatDisabled?.[f] ?? null,
      })),
    });
  }
  if (a.onReveal) {
    if (out.length) out.push({ kind: 'separator' });
    out.push({ id: 'reveal', label: 'Open in Explorer', onSelect: a.onReveal });
  }
  return out;
}

function Item({ menu, item }: { menu: string; item: MenuItem }) {
  return (
    <Menu.Item
      className="pf-menu-item"
      data-testid={`${menu}-${item.id}`}
      disabled={!!item.disabled}
      data-reason={item.disabled || undefined}
      title={item.disabled || undefined}
      onSelect={item.onSelect}
    >
      {item.label}
      {item.hint && <span className="pf-menu-hint">{item.hint}</span>}
    </Menu.Item>
  );
}

function Entries({ menu, entries }: { menu: string; entries: MenuEntry[] }) {
  return (
    <>
      {entries.map((e, i) => {
        if (e.kind === 'separator')
          return <Menu.Separator key={`sep-${i}`} className="pf-menu-sep" />;
        if (e.kind === 'sub')
          return (
            <Menu.Sub key={e.id}>
              <Menu.SubTrigger
                className="pf-menu-item"
                data-testid={`${menu}-${e.id}`}
                disabled={!!e.disabled}
                data-reason={e.disabled || undefined}
                title={e.disabled || undefined}
              >
                {e.label}
                <span className="pf-menu-hint" aria-hidden="true">
                  ▸
                </span>
              </Menu.SubTrigger>
              <Menu.Portal>
                <Menu.SubContent className="pf-menu" data-testid={`${menu}-${e.id}-menu`}>
                  {e.items.map((it) => (
                    <Item key={it.id} menu={menu} item={it} />
                  ))}
                </Menu.SubContent>
              </Menu.Portal>
            </Menu.Sub>
          );
        return <Item key={e.id} menu={menu} item={e} />;
      })}
    </>
  );
}

/** A right-click menu on `children` (the trigger, rendered as is). No entries → no menu. */
export function ContextMenu({
  testId,
  itemPrefix,
  entries,
  children,
  disabled,
  onOpenChange,
}: {
  testId: string;
  /** Item test ids are `<itemPrefix>-<id>`; default `testId`. */
  itemPrefix?: string;
  entries: MenuEntry[];
  children: ReactElement;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <Menu.Root onOpenChange={onOpenChange}>
      <Menu.Trigger asChild disabled={disabled || entries.length === 0}>
        {children}
      </Menu.Trigger>
      <Menu.Portal>
        <Menu.Content className="pf-menu" data-testid={testId}>
          <Entries menu={itemPrefix ?? testId} entries={entries} />
        </Menu.Content>
      </Menu.Portal>
    </Menu.Root>
  );
}
