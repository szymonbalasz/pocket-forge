// Radix tooltip around a control. Disabled buttons don't fire pointer events, so the trigger is
// a wrapping span; `data-tooltip` mirrors the text for tests and screen readers.
import { Tooltip } from 'radix-ui';
import type { ReactNode } from 'react';

export function Tip({
  text,
  side = 'right',
  children,
}: {
  text: string;
  side?: 'top' | 'right' | 'bottom' | 'left';
  children: ReactNode;
}) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>
        <span className="edt-tip" data-tooltip={text}>
          {children}
        </span>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="pf-tooltip" side={side} sideOffset={6}>
          {text}
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
