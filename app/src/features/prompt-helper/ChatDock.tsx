// The prompt chat dock (#61, FR-PRM-21): a panel in the main window, left of the right-hand pane.
// Opened by the prompt block's Chat button or the status-bar toggle, resized by its left-edge
// handle (DOCK_MIN–DOCK_MAX), and remembered per machine. Once opened it stays mounted while
// hidden, so a reply keeps streaming into it (hiding never cancels a turn, FR-PRM-35).
import { useCallback, useEffect, useRef, useState } from 'react';

import { ChatPanel } from './ChatPanel';
import {
  commitDockWidth,
  DOCK_MAX,
  DOCK_MIN,
  loadDock,
  previewDockWidth,
  setDockOpen,
  toggleDock,
  useDock,
} from './dock';
import './chat.css';

export function ChatDock() {
  const { open, width } = useDock();
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    void loadDock();
  }, []);
  useEffect(() => {
    if (open) setMounted(true);
  }, [open]);

  const start = useRef<{ x: number; w: number } | null>(null);
  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      start.current = { x: e.clientX, w: width };
      (e.target as Element).setPointerCapture?.(e.pointerId);
    },
    [width],
  );
  // The handle is on the left edge: dragging left widens the dock.
  const onPointerMove = useCallback((e: React.PointerEvent) => {
    const s = start.current;
    if (s) previewDockWidth(s.w - (e.clientX - s.x));
  }, []);
  const onPointerUp = useCallback(() => {
    if (!start.current) return;
    start.current = null;
    commitDockWidth();
  }, []);

  if (!mounted) return null;
  return (
    <aside
      className="prm-dock"
      data-testid="prm-dock"
      aria-label="Prompt chat"
      hidden={!open}
      style={{ width }}
    >
      <div
        className="prm-dock-resize"
        role="separator"
        aria-orientation="vertical"
        aria-valuemin={DOCK_MIN}
        aria-valuemax={DOCK_MAX}
        aria-valuenow={width}
        data-testid="prm-dock-resize"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      />
      <ChatPanel visible={open} onClose={() => setDockOpen(false)} />
    </aside>
  );
}

/** The status-bar toggle (#61): opens or closes the chat dock from anywhere. */
export function ChatDockToggle() {
  const { open } = useDock();
  useEffect(() => {
    void loadDock();
  }, []);
  return (
    <button
      type="button"
      className={`prm-dock-toggle${open ? ' is-on' : ''}`}
      data-testid="prm-dock-toggle"
      aria-pressed={open}
      title={open ? 'Hide the prompt chat' : 'Show the prompt chat'}
      onClick={toggleDock}
    >
      Chat
    </button>
  );
}
