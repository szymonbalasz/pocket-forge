// PRM slice 5: the chat context sink (FR-PRM-23). The visible tab's prompt block publishes its
// draft input through `shared/chatContext`; this forwards it to the core for the open project
// (`prm_context_publish` / `prm_context_clear`). Installed once for the main window, so a tab
// switch's clear and the next tab's publish both reach the core in order. Failures are dropped:
// the next publish (or the flush before a send) tries again.
import { api } from '../../api';
import { chatContext } from '../../shared/chatContext';
import { projectStore } from '../../store/project';

const projectId = (): string | null => projectStore.get().project?.project_id ?? null;

export function installChatSink(): void {
  chatContext.setSink({
    publish(host, input) {
      const pid = projectId();
      return pid ? api.chat.contextPublish(pid, host, input).catch(() => undefined) : undefined;
    },
    clear(host) {
      const pid = projectId();
      return pid ? api.chat.contextClear(pid, host).catch(() => undefined) : undefined;
    },
  });
}
