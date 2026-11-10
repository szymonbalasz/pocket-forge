import type { Feature } from '../../shared/feature';
import { ExportPanel, ExportStatus } from './ExportPanel';

export const exportFeature: Feature = {
  id: 'export',
  prefix: 'exp',
  // Right panel, under Generate (RGN registers first, ui-guidelines § Layout).
  slots: { right: ExportPanel },
  // After the project's save state (50), before the provider chip (90).
  status: [{ id: 'progress', priority: 60, drop: 1, Component: ExportStatus }],
};
