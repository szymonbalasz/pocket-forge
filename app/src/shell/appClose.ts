// R101: pressing the window X or quitting the app is a project close. The Tauri layer holds the
// close and sends `app:close-requested`; this runs the same close guards as switching projects
// (EDT's last mask submit, the export question, PRJ's `unsaved`) and exits only once the project
// closed. A refused or cancelled close keeps the app open. R110: the app exits 10 s after the
// ack anyway unless a close question is on screen, so this reports the question showing and going,
// and a dismissed quit.
import {
  appCloseAck,
  appCloseAsking,
  appCloseGuard,
  appCloseUnguard,
  appExit,
  onEvent,
} from '../api';
import { projectStore } from '../store/project';

/** The quit under way: a later close request shares it (R109), so it settles with the first. */
let pending: Promise<void> | null = null;
/** The close was refused with `unsaved`: "Close anyway" still ends in an exit, Dismiss doesn't. */
let awaitingForce = false;
/** The close question state last told to the app (R110). */
let reported = false;

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
    // Refused or kept (Keep exporting): the quit is off (R110).
    if (!awaitingForce) void appCloseAsking(false, true);
    return;
  }
  await appExit();
}

/** R110: tells the app when a close question shows or goes. */
function report(): void {
  const now = projectStore.closeAsking();
  if (now === reported) return;
  reported = now;
  void appCloseAsking(now);
}

/** Follows the project after an `unsaved` refusal: closed → exit; the question gone → stay. */
function follow(): void {
  if (!awaitingForce || pending) return;
  const s = projectStore.get();
  if (!s.project) {
    awaitingForce = false;
    void appExit();
  } else if (s.error?.code !== 'unsaved') {
    // Dismissed: the quit is off (R110).
    awaitingForce = false;
    void appCloseAsking(false, true);
  }
}

/** Listens for close requests and tells the app the guards are in place. Returns the removal,
 *  which tells the app they're gone (R109: a crash unmounts the root, and X must still close). */
export function installAppClose(): () => void {
  reported = false;
  const unsubscribe = projectStore.subscribe(() => {
    report();
    follow();
  });
  const listening = onEvent('app:close-requested', () => {
    // R109: acked at once (the app closes anyway if no ack comes within 3 s, R110: or if the
    // close doesn't end within 10 s while no question shows), and run only if the app sent it:
    // another window's copy is ignored.
    void appCloseAck().then((asked) => (asked ? quit() : undefined));
  });
  const guarded = listening.then(
    () => appCloseGuard().then(() => true),
    () => false, // No core (a plain browser): nothing to guard.
  );
  return () => {
    unsubscribe();
    awaitingForce = false;
    void listening.then(
      (off) => off(),
      () => undefined,
    );
    void guarded.then((was) => (was ? appCloseUnguard() : undefined));
  };
}
