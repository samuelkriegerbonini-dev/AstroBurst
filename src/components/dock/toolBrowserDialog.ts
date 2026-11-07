import { firstRendered } from "./dockGeometry";

export function openToolBrowserDialog(
  dialog: Pick<HTMLDialogElement, "showModal" | "close">,
  input: Pick<HTMLElement, "focus"> | null,
  opener: (Pick<HTMLElement, "focus" | "isConnected"> & { getClientRects(): ArrayLike<unknown> }) | null,
  fallback: () => Pick<HTMLElement, "focus"> | null,
): () => void {
  dialog.showModal();
  input?.focus();
  return () => {
    dialog.close();
    const target = opener?.isConnected && opener !== document.body && firstRendered([opener]) ? opener : fallback();
    target?.focus({ preventScroll: true });
  };
}
