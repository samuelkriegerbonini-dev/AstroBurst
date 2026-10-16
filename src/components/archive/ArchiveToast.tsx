import { useEffect } from "react";
import type { ArchiveController } from "./state/archiveController";

const TOAST_MS = 2400;

interface ArchiveToastProps {
  toast: { text: string; seq: number } | null;
  ctl: Pick<ArchiveController, "dismissToast">;
}

export default function ArchiveToast({ toast, ctl }: ArchiveToastProps) {
  const seq = toast?.seq ?? null;
  useEffect(() => {
    if (seq === null) return;
    const id = setTimeout(() => ctl.dismissToast(), TOAST_MS);
    return () => clearTimeout(id);
  }, [seq, ctl]);
  if (!toast) return null;
  return (
    <div key={toast.seq} data-testid="mast-toast" role="status" className="ab-mast-toast">
      {toast.text}
    </div>
  );
}
