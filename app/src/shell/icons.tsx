// Inline SVG icons (ui-guidelines: no icon package). 16 px, stroke = currentColor, so they take
// the button's text colour. Add icons here. Features may import this one shell file (ESLint
// boundaries allow it); everything else in shell/ stays off-limits to them.
import type { SVGProps } from 'react';

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function make(paths: React.ReactNode, name: string) {
  function Icon({ size = 16, ...rest }: IconProps) {
    return (
      <svg
        width={size}
        height={size}
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.4}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        {...rest}
      >
        {paths}
      </svg>
    );
  }
  Icon.displayName = `Icon${name}`;
  return Icon;
}

export const IconPan = make(
  <path d="M5.5 8V3.5a1 1 0 0 1 2 0V7m0-4.5v-.5a1 1 0 0 1 2 0V7m0-4a1 1 0 0 1 2 0V8m0-2.5a1 1 0 0 1 2 0V10a4.5 4.5 0 0 1-4.5 4.5H8.6a4 4 0 0 1-3.2-1.6L2.7 9.3a1 1 0 0 1 1.5-1.3L5.5 9.3" />,
  'Pan',
);
export const IconRegion = make(
  <rect x="2.5" y="3.5" width="11" height="9" rx="0.5" strokeDasharray="2 1.6" />,
  'Region',
);
export const IconBrush = make(
  <>
    <path d="M13.5 2.5 7.2 8.8" />
    <path d="M7.4 8.6a2 2 0 0 0-2.8 0c-.8.8-.5 2-1.1 2.9-.4.6-1 .9-1 .9s2.9.6 4.6-1a2 2 0 0 0 .3-2.8Z" />
  </>,
  'Brush',
);
export const IconEraser = make(
  <>
    <path d="M6.5 13.5 2.6 9.6a1.2 1.2 0 0 1 0-1.7l5.3-5.3a1.2 1.2 0 0 1 1.7 0l3.8 3.8a1.2 1.2 0 0 1 0 1.7l-5.4 5.4Z" />
    <path d="M5 6.5 10 11.5M6.5 13.5h7" />
  </>,
  'Eraser',
);
export const IconFill = make(
  <>
    <rect x="3" y="3" width="10" height="10" rx="1" fill="currentColor" fillOpacity={0.35} />
  </>,
  'Fill',
);
export const IconClear = make(
  <>
    <rect x="3" y="3" width="10" height="10" rx="1" />
    <path d="M5.5 5.5l5 5m0-5-5 5" />
  </>,
  'Clear',
);
export const IconUndo = make(<path d="M6 4 3 7l3 3M3 7h6.5a3.5 3.5 0 0 1 0 7H8" />, 'Undo');
export const IconRedo = make(<path d="m10 4 3 3-3 3m3-3H6.5a3.5 3.5 0 0 0 0 7H8" />, 'Redo');
export const IconMask = make(
  <>
    <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8s-2.4 4.5-6.5 4.5S1.5 8 1.5 8Z" />
    <circle cx="8" cy="8" r="2" />
  </>,
  'Mask',
);
export const IconMaps = make(
  <path d="M1.5 3.5 5.5 2l5 1.5 4-1.5v10.5l-4 1.5-5-1.5-4 1.5V3.5ZM5.5 2v10.5M10.5 3.5V14" />,
  'Maps',
);
export const IconTokens = make(
  <>
    <circle cx="8" cy="8" r="6" />
    <circle cx="8" cy="6.5" r="2" />
    <path d="M4.5 12.3a4 4 0 0 1 7 0" />
  </>,
  'Tokens',
);
export const IconItems = make(
  <path d="M8 1.5 13.5 4.5v7L8 14.5l-5.5-3v-7L8 1.5Zm0 0v13M2.5 4.5 8 7.5l5.5-3" />,
  'Items',
);
export const IconReferences = make(
  <>
    <rect x="2" y="4.5" width="9" height="9" rx="1" />
    <path d="M5 4.5v-2h9v9h-3" />
    <path d="m2.5 12 2.5-3 2 2 1.5-1.5 2.5 2.5" />
  </>,
  'References',
);
export const IconSettings = make(
  <>
    <circle cx="8" cy="8" r="2" />
    <path d="M8 1.5v2m0 9v2m6.5-6.5h-2m-9 0h-2m11.1-4.6-1.4 1.4m-6.4 6.4-1.4 1.4m9.2 0-1.4-1.4M4.8 4.8 3.4 3.4" />
  </>,
  'Settings',
);
export const IconStar = make(
  <path d="m8 1.8 1.9 3.9 4.3.6-3.1 3 .7 4.3L8 11.6l-3.8 2 .7-4.3-3.1-3 4.3-.6L8 1.8Z" />,
  'Star',
);
export const IconCheck = make(<path d="m3 8.5 3 3 7-7" />, 'Check');
export const IconTrash = make(
  <path d="M2.5 4h11M6 4V2.5h4V4m-6 0 .7 9.5h6.6L12 4M6.8 6.5v5m2.4-5v5" />,
  'Trash',
);
export const IconFit = make(<path d="M2 6V2h4M10 2h4v4M14 10v4h-4M6 14H2v-4" />, 'Fit');
export const IconCompare = make(
  <>
    <rect x="2" y="3" width="12" height="10" rx="1" />
    <path d="M8 1.5v13" />
  </>,
  'Compare',
);
export const IconPlus = make(<path d="M8 3v10M3 8h10" />, 'Plus');
