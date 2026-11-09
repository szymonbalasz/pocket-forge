// UI feature registry (AGENTS §9). **Append-only**: add one import and one entry per feature.
// A feature lives in features/<feature>/ and is imported only through its index.ts.
import type { Feature } from '../shared/feature';
import { editorFeature } from '../features/editor';
import { libraryFeature } from '../features/library';
import { projectFeature } from '../features/project';
import { providerFeature } from '../features/provider';
import { regionEditFeature } from '../features/region-edit';
import { exportFeature } from '../features/export';
import { referencesFeature } from '../features/references';
import { newAssetGenerationFeature } from '../features/new-asset-generation';

export const features: readonly Feature[] = [
  libraryFeature,
  editorFeature,
  projectFeature,
  providerFeature,
  regionEditFeature,
  exportFeature,
  referencesFeature,
  newAssetGenerationFeature,
];
