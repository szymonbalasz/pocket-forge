// A role select (Radix Select, ui-guidelines) for pack images (FR-REF-11) and block items
// (FR-REF-19). Radix items can't have an empty value, so "no role" is `none`.
import { Select } from 'radix-ui';

import type { RefRole } from '../../api';
import { roleLabel } from './format';

const NONE = 'none';

export function RoleSelect({
  value,
  roles,
  allowNone,
  disabled,
  disabledRoles = [],
  testId,
  label,
  onChange,
}: {
  value: RefRole | null;
  roles: readonly RefRole[];
  allowNone: boolean;
  disabled?: boolean;
  /** Roles whose slot is full (FR-REF-18). */
  disabledRoles?: readonly RefRole[];
  testId: string;
  label: string;
  onChange(role: RefRole | null): void;
}) {
  return (
    <Select.Root
      value={value ?? NONE}
      disabled={disabled}
      onValueChange={(v) => onChange(v === NONE ? null : (v as RefRole))}
    >
      <Select.Trigger
        className="ref-select"
        aria-label={label}
        data-testid={testId}
        data-value={value ?? NONE}
      >
        <Select.Value />
        <Select.Icon className="muted">▾</Select.Icon>
      </Select.Trigger>
      <Select.Portal>
        <Select.Content className="ref-select-content" position="popper" sideOffset={2}>
          <Select.Viewport>
            {allowNone && (
              <Select.Item
                className="ref-select-item"
                value={NONE}
                data-testid={`${testId}-option`}
              >
                <Select.ItemText>{roleLabel(null)}</Select.ItemText>
              </Select.Item>
            )}
            {roles.map((r) => (
              <Select.Item
                key={r}
                className="ref-select-item"
                value={r}
                disabled={r !== value && disabledRoles.includes(r)}
                data-testid={`${testId}-option`}
                data-record-id={r}
              >
                <Select.ItemText>{roleLabel(r)}</Select.ItemText>
              </Select.Item>
            ))}
          </Select.Viewport>
        </Select.Content>
      </Select.Portal>
    </Select.Root>
  );
}
