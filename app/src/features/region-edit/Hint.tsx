// A Radix tooltip on a small element (ui-guidelines: Radix tips, never a native `title`).
import { Tooltip } from 'radix-ui';
import type { ReactElement } from 'react';

export function Hint({
  text,
  side = 'top',
  children,
}: {
  text: string;
  side?: 'top' | 'right' | 'bottom' | 'left';
  /** One element: the trigger (it gets `data-tooltip` for tests and a11y tools). */
  children: ReactElement;
}) {
  return (
    <Tooltip.Provider delayDuration={400}>
      <Tooltip.Root>
        <Tooltip.Trigger asChild data-tooltip={text}>
          {children}
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content className="pf-tooltip" side={side} sideOffset={6}>
            {text}
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}
