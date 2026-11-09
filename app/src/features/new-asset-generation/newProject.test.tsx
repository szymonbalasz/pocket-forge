// Generate new… (FR-GEN-01, BL-40, R85/R86): the dialog's two requests (start on the New image tab,
// focus the next step) are for the project it creates; an open that fails or is cancelled must
// not hand them to the next project opened.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';

import { installTestCore } from '../../api/mocks/testCore';
import { generateTab } from '../../shared/generateTabs';
import { NewProjectDialog } from './NewProjectDialog';
import { gen } from './store';

const core = installTestCore();

afterEach(() => {
  cleanup();
  gen.takeFocusRequest();
  generateTab.openNextWith(null);
  generateTab.select('region');
});

async function create(opened: boolean) {
  core.useHandlers({ gen_project_create: () => ({ path: 'C:/pf-test/library/Maps/Crypt' }) });
  const onCreated = vi.fn(() => Promise.resolve(opened));
  render(
    <NewProjectDialog
      open
      section="maps"
      taken={[]}
      onOpenChange={() => undefined}
      onCreated={onCreated}
    />,
  );
  await act(async () => fireEvent.click(await screen.findByTestId('gen-new-ok')));
  await waitFor(() => expect(onCreated).toHaveBeenCalledWith('C:/pf-test/library/Maps/Crypt'));
  await act(() => Promise.resolve());
}

test('FR-GEN-01 BL-40 the created project opens on New image with the focus request', async () => {
  await create(true);
  expect(gen.focusRequested()).toBe(true);
  generateTab.projectOpened('01JPROJECTCRYPT00000000000');
  expect(generateTab.get()).toBe('new-image');
});

test('R85 R86 an open that failed or was cancelled hands neither the tab nor the focus to the next project', async () => {
  await create(false);
  expect(gen.focusRequested()).toBe(false);
  generateTab.projectOpened('01JOTHERPROJECT00000000000');
  expect(generateTab.get()).toBe('region');
});
