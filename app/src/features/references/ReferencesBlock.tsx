// The References block (reference-packs §7, FR-REF-15..20, 27): rendered by RGN in its Generate
// section between Model and Prompt, through this feature's index.ts (R44). Header with the count
// per slot · "This project" then each picked pack · items with a tick, a role where the model
// exposes roles (FR-REF-19) and badges · + Pack (Radix Popover multi-select).
import { useEffect, useState } from 'react';
import { Popover } from 'radix-ui';

import type { DraftReferences, RefItem, RefResolved, RefRole } from '../../api';
import { block, useBlock } from './blockStore';
import {
  badgeLabel,
  headerCount,
  overLimit,
  roleLabel,
  roleMode,
  slotName,
  slotOf,
  tickBlock,
  underMin,
} from './format';
import { RoleSelect } from './RoleSelect';
import './references.css';

export interface ReferencesBlockProps {
  projectId: string | null;
  /** The draft's `references` when the project opened (schema §4.5). */
  draftReferences: DraftReferences | null | undefined;
  /** The selected model, once its capabilities are loaded. */
  modelId: string | null;
  /** The model's `slots[]` is non-empty (FR-REF-15). */
  hasSlots: boolean;
  /** Read-only project: shown but disabled (FR-REF-27). */
  readOnly: boolean;
  /** Called after each change; `save` = a user edit to write with the draft (FR-REF-20). */
  onChange(save: boolean): void;
}

export function ReferencesBlock(props: ReferencesBlockProps) {
  const { projectId, draftReferences, modelId, hasSlots, onChange } = props;
  const active = hasSlots ? modelId : null;
  useEffect(() => {
    block.onChange(onChange);
  }, [onChange]);
  useEffect(() => {
    block.attach(projectId, draftReferences ?? null);
    // The draft is read once per project; later edits live in the block store.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);
  useEffect(() => {
    block.setModel(projectId ? active : null);
  }, [projectId, active]);

  if (!projectId || !active) return null;
  return <BlockView readOnly={props.readOnly} />;
}

function BlockView({ readOnly: projectReadOnly }: { readOnly: boolean }) {
  const s = useBlock();
  const r = s.resolved;
  // The core hides the block for a model without slots too (FR-REF-15).
  if (r && !r.visible) return null;
  const readOnly = projectReadOnly || !!r?.read_only;
  return (
    <div
      className="rgn-section ref-block"
      data-testid="ref-block"
      aria-disabled={readOnly || undefined}
      aria-busy={s.loading || undefined}
    >
      <div className="ref-block-head">
        <strong>References</strong>
        {r && r.slots.length > 0 && (
          <span className="tabular small" data-testid="ref-block-count">
            {r.slots.length === 1
              ? headerCount(r.slots).replace(/^References /, '')
              : headerCount(r.slots)}
          </span>
        )}
        {r && roleMode(r.slots) === 'label' && (
          <span className="pf-chip" data-testid="ref-block-role">
            {roleLabel(r.slots[0]!.role)}
          </span>
        )}
        <span className="pf-spacer" />
        <PackPicker disabled={readOnly} />
      </div>
      {r && overLimit(r) && (
        <p className="muted small tabular" data-testid="ref-block-over">
          {overLimit(r)}
        </p>
      )}
      {r &&
        underMin(r.slots).map((slot) => (
          <p
            key={slot.name}
            className="ref-block-under small"
            data-testid="ref-block-under"
            data-record-id={slot.name}
          >
            {slotName(slot)} needs at least {slot.min}
          </p>
        ))}
      {s.error && (
        <p className="error small" data-testid="ref-block-error" data-code={s.error.code}>
          References unavailable: {s.error.message}
        </p>
      )}
      {!r && s.loading && (
        <div className="ref-block-list" data-testid="ref-block-loading">
          <div className="ref-skeleton" />
          <div className="ref-skeleton" />
        </div>
      )}
      {r && <Groups r={r} readOnly={readOnly} />}
      {s.notices.length > 0 && (
        <div className="rgn-warning small" role="status">
          <ul>
            {s.notices.map((n, i) => (
              <li key={i} data-testid="ref-block-notice" data-code={n.code}>
                {n.message}
              </li>
            ))}
          </ul>
          <button type="button" className="link small" onClick={() => block.dismissNotices()}>
            Dismiss
          </button>
        </div>
      )}
    </div>
  );
}

function Groups({ r, readOnly }: { r: RefResolved; readOnly: boolean }) {
  const { refs } = useBlock();
  if (r.items.length === 0 && !refs?.packs.length)
    return (
      <p className="muted small" data-testid="ref-block-empty">
        No references. Add files to this project’s references folder or pick a pack.
      </p>
    );
  const project = r.items.filter((i) => i.source === 'project');
  return (
    <>
      {project.length > 0 && (
        <Group id="project" title="This project" items={project} r={r} readOnly={readOnly} />
      )}
      {(refs?.packs ?? []).map((p) => {
        const items = r.items.filter((i) => i.pack_id === p.pack_id);
        return (
          <Group
            key={p.pack_id}
            id={p.pack_id}
            title={items[0]?.pack_name ?? p.name}
            items={items}
            r={r}
            readOnly={readOnly}
            onUnpick={() => block.unpickPack(p.pack_id)}
          />
        );
      })}
    </>
  );
}

function Group({
  id,
  title,
  items,
  r,
  readOnly,
  onUnpick,
}: {
  id: string;
  title: string;
  items: RefItem[];
  r: RefResolved;
  readOnly: boolean;
  onUnpick?: () => void;
}) {
  return (
    <>
      <div className="ref-block-group" data-testid="ref-block-group" data-record-id={id}>
        <span>{title}</span>
        {onUnpick && (
          <button
            type="button"
            aria-label={`Remove ${title}`}
            title="Remove this pack"
            data-testid="ref-block-pack-unpick"
            disabled={readOnly}
            onClick={onUnpick}
          >
            ×
          </button>
        )}
      </div>
      {items.length === 0 ? (
        <p className="muted small">No images in this pack.</p>
      ) : (
        <ul className="ref-block-list">
          {items.map((i) => (
            <Item key={i.key} item={i} r={r} readOnly={readOnly} />
          ))}
        </ul>
      )}
    </>
  );
}

function Item({ item, r, readOnly }: { item: RefItem; r: RefResolved; readOnly: boolean }) {
  const [failed, setFailed] = useState(false);
  const blocked = tickBlock(r.slots, item);
  const mode = roleMode(r.slots);
  const roles = r.slots.flatMap((s) => (s.role ? [s.role] : []));
  // A ticked item can't move into a full slot (FR-REF-18).
  const full: RefRole[] = item.on
    ? r.slots
        .filter((s) => s.role && s.on >= s.max && s !== slotOf(r.slots, item))
        .map((s) => s.role!)
    : [];
  return (
    <li
      className="ref-block-item"
      data-testid="ref-block-item"
      data-record-id={item.key}
      data-on={item.on ? 'true' : 'false'}
      data-blocked={blocked ? 'true' : undefined}
    >
      <label title={blocked ?? item.file}>
        <input
          type="checkbox"
          data-testid="ref-block-item-check"
          checked={item.on}
          disabled={readOnly || !!blocked}
          onChange={() => block.toggle(item.key)}
        />
        {item.thumb_url && !failed ? (
          <img src={item.thumb_url} alt="" loading="lazy" onError={() => setFailed(true)} />
        ) : (
          <span className="ref-block-thumb-empty" />
        )}
        <span className="ref-block-file">{item.file}</span>
      </label>
      {item.badges.map((b) => (
        <span
          key={b}
          className={`ref-badge ref-badge-${b.split(':')[0]}`}
          data-testid="ref-block-item-badge"
          data-badge={b}
        >
          {badgeLabel(b)}
        </span>
      ))}
      {blocked && !item.on && blocked.startsWith('limit') && (
        <span className="muted small" data-testid="ref-block-item-limit">
          {blocked}
        </span>
      )}
      {mode === 'select' && (
        <RoleSelect
          value={item.role ?? null}
          roles={roles}
          allowNone={false}
          disabledRoles={full}
          disabled={readOnly}
          label={`Role of ${item.file}`}
          testId="ref-block-item-role"
          onChange={(role) => block.setRole(item.key, role)}
        />
      )}
    </li>
  );
}

/** + Pack (Radix Popover, ui-guidelines): a multi-select over the packs. A `not_ready` pack can't
 *  be attached until a later listing adopts it (FR-REF-04). */
function PackPicker({ disabled }: { disabled: boolean }) {
  const { packs, refs } = useBlock();
  const picked = new Set((refs?.packs ?? []).map((p) => p.pack_id));
  return (
    <Popover.Root onOpenChange={(open) => open && void block.loadPacks()}>
      <Popover.Trigger asChild>
        <button
          type="button"
          className="small"
          data-testid="ref-block-pack-picker"
          disabled={disabled}
        >
          + Pack
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          className="ref-popover"
          align="end"
          sideOffset={4}
          data-testid="ref-block-pack-list"
        >
          {packs === null && <p className="muted small">Loading packs…</p>}
          {packs?.length === 0 && (
            <p className="muted small">No packs yet. Create one under References in the rail.</p>
          )}
          {packs?.map((p) => {
            const id = p.pack_id;
            const notReady = !id || p.badges.includes('not_ready');
            const on = !!id && picked.has(id);
            return (
              <label
                key={p.id}
                data-testid="ref-block-pack-option"
                data-record-id={p.id}
                data-disabled={notReady && !on ? 'true' : undefined}
              >
                <input
                  type="checkbox"
                  checked={on}
                  disabled={notReady && !on}
                  onChange={() => (on && id ? block.unpickPack(id) : block.pickPack(p))}
                />
                <span className="ref-popover-name" title={p.name}>
                  {p.name}
                </span>
                <span className="pf-spacer" />
                <span className="muted small tabular ref-popover-count">
                  {notReady ? 'not ready' : p.image_count}
                </span>
              </label>
            );
          })}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
