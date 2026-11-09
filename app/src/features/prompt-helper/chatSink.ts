// PRM slice 5: the chat context sink (FR-PRM-23). The visible tab's prompt block publishes its
// draft input through `shared/chatContext`; this forwards it to the core for the open project
// (`prm_context_publish` / `prm_context_clear`). Installed once for the main window, so a tab
// switch's clear and the next tab's publish both reach the core in order. Failures are dropped:
// the next publish (or the flush before a send) tries again.
import { api, currentWindowLabel, emitEvent, onEvent } from '../../api';
import { chatContext } from '../../shared/chatContext';
import { projectStore } from '../../store/project';

const projectId = (): string | null => projectStore.get().project?.project_id ?? null;

/** BL-42: the chat window asks before each send; this window publishes what is pending and
 *  answers with the same id. Only the main window answers. */
function answerFlushRequests(): void {
  if (currentWindowLabel() === 'prompt-chat') return;
  void onEvent('prm:context-flush', (p) => {
    void chatContext
      .flush()
      .then(() => emitEvent('prm:context-flushed', { id: p.id }))
      .catch(() => undefined);
  }).catch(() => undefined);
}

export function installChatSink(): void {
  answerFlushRequests();
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
