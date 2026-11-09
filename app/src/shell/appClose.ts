// R101: pressing the window X or quitting the app is a project close. The Tauri layer holds the
// close and sends `app:close-requested`; this runs the same close guards as switching projects
// (EDT's last mask submit, the export question, PRJ's `unsaved`) and exits only once the project
// closed. A refused or cancelled close keeps the app open.
import { appCloseGuard, appExit, onEvent } from '../api';
import { projectStore } from '../store/project';

/** The quit under way: a later close request shares it (R109), so it settles with the first. */
let pending: Promise<void> | null = null;
/** The close was refused with `unsaved`: "Close anyway" still ends in an exit, Dismiss doesn't. */
let awaitingForce = false;

/** One close request: closes the open project through its guards, then exits. A request while
 *  one runs gets the same promise. */
export function quit(): Promise<void> {
  pending ??= quitNow().finally(() => {
    pending = null;
  });
  return pending;
}

async function quitNow(): Promise<void> {
  awaitingForce = false;
  if (projectStore.get().project) await projectStore.close();
  const s = projectStore.get();
  if (s.project) {
    awaitingForce = s.error?.code === 'unsaved';
    return;
  }
  await appExit();
}

/** Follows the project after an `unsaved` refusal: closed → exit; the question gone → stay. */
function follow(): void {
  if (!awaitingForce || pending) return;
  const s = projectStore.get();
  if (!s.project) {
    awaitingForce = false;
    void appExit();
  } else if (s.error?.code !== 'unsaved') {
    awaitingForce = false;
  }
}

/** Listens for close requests and tells the app the guards are in place. Returns the removal. */
export function installAppClose(): () => void {
  const unsubscribe = projectStore.subscribe(follow);
  const listening = onEvent('app:close-requested', () => void quit());
  void listening.then(
    () => appCloseGuard(),
    () => undefined, // No core (a plain browser): nothing to guard.
  );
  return () => {
    unsubscribe();
    awaitingForce = false;
    void listening.then(
      (off) => off(),
      () => undefined,
    );
  };
}
