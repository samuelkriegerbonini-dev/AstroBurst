import { useEffect } from "react";
import { useHistory } from "../../hooks/useHistory";
import { appModeStore } from "../../hooks/useAppMode";
import { useCompositeMode } from "../../hooks/useCompositeMode";
import { historyKeyAction, isMacPlatform, openMenuPresent } from "../../utils/historyKeys";

const MENU_SELECTOR = "[data-dock-menu], [aria-modal='true']";

function typingTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable);
}

export default function HistoryShortcuts(): null {
  const { undo, redo } = useHistory();
  const compositeMode = useCompositeMode();

  useEffect(() => {
    const mac = isMacPlatform(navigator.platform);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat) return;
      const action = historyKeyAction(e, {
        inInput: typingTarget(e.target),
        menuOpen: openMenuPresent(document.querySelectorAll(MENU_SELECTOR)),
        mode: appModeStore.get(),
        compositeMode,
        mac,
      });
      if (!action) return;
      e.preventDefault();
      void (action === "undo" ? undo() : redo());
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [undo, redo, compositeMode]);

  return null;
}
