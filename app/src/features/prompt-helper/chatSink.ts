// PRM slice 5: the chat context sink (FR-PRM-23). The visible tab's prompt block publishes its
// draft input through `shared/chatContext`; this forwards it to the core for the open project
// (`prm_context_publish` / `prm_context_clear`). Installed once, so a tab switch's clear and the
// next tab's publish both reach the core in order. Failures are dropped: the next publish (or
// the flush before a send) tries again. #61: the chat is a panel in the same window, so it hears
// each publish here (R103) instead of through a window-to-window event.
import { api } from '../../api';
import { chatContext, type ChatHostId } from '../../shared/chatContext';
import { projectStore } from '../../store/project';

const projectId = (): string | null => projectStore.get().project?.project_id ?? null;

type Published = (projectId: string, host: ChatHostId) => void;
const published = new Set<Published>();

/** R103: called after each publish reached the core (the chat's next-turn strip re-reads). */
export function onContextPublished(l: Published): () => void {
  published.add(l);
  return () => published.delete(l);
}

export function installChatSink(): void {
  chatContext.setSink({
    publish(host, input) {
      const pid = projectId();
      if (!pid) return undefined;
      return api.chat.contextPublish(pid, host, input).then(
        () => published.forEach((l) => l(pid, host)),
        () => undefined,
      );
    },
    clear(host) {
      const pid = projectId();
      return pid ? api.chat.contextClear(pid, host).catch(() => undefined) : undefined;
    },
  });
}
