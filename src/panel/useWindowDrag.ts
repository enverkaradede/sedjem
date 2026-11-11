import { LogicalPosition } from "@tauri-apps/api/dpi";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useRef, type PointerEvent } from "react";

interface DragState {
  pointerX: number;
  pointerY: number;
  /** Window origin in logical points; null until the async lookup resolves. */
  origin: { x: number; y: number } | null;
  target: { x: number; y: number } | null;
  frame: number | null;
}

/**
 * Moves the panel window by dragging an element. Native window dragging is
 * unreliable for a non-activating NSPanel, so the position is set directly,
 * coalesced to one IPC call per animation frame. Screen coordinates (not
 * client ones) are used so the moving window never feeds back into the math.
 */
export function useWindowDrag() {
  const drag = useRef<DragState | null>(null);

  const flush = useCallback(() => {
    const state = drag.current;
    if (!state) return;
    state.frame = null;
    if (!state.target) return;
    const { x, y } = state.target;
    getCurrentWindow()
      .setPosition(new LogicalPosition(x, y))
      .catch((err: unknown) => console.error("panel move failed", err));
  }, []);

  const onPointerDown = useCallback((event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const state: DragState = { pointerX: event.screenX, pointerY: event.screenY, origin: null, target: null, frame: null };
    drag.current = state;
    const win = getCurrentWindow();
    Promise.all([win.outerPosition(), win.scaleFactor()])
      .then(([position, scale]) => {
        const logical = position.toLogical(scale);
        state.origin = { x: logical.x, y: logical.y };
      })
      .catch((err: unknown) => console.error("panel position lookup failed", err));
  }, []);

  const onPointerMove = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      const state = drag.current;
      if (!state?.origin) return;
      state.target = {
        x: state.origin.x + (event.screenX - state.pointerX),
        y: state.origin.y + (event.screenY - state.pointerY),
      };
      state.frame ??= requestAnimationFrame(flush);
    },
    [flush],
  );

  const end = useCallback(() => {
    const state = drag.current;
    if (!state) return;
    if (state.frame !== null) {
      cancelAnimationFrame(state.frame);
      flush(); // land exactly where the pointer was released
    }
    drag.current = null;
  }, [flush]);

  return { onPointerDown, onPointerMove, onPointerUp: end, onPointerCancel: end, onLostPointerCapture: end };
}
