import { memo, useState } from "react";
import type { ReactNode } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useMeasurementLog } from "../../hooks/useMeasurementLog";

let openForSession = false;

function AnalysisLogFooter({ children }: { children: ReactNode }) {
  const count = useMeasurementLog().length;
  const [open, setOpen] = useState(openForSession);

  const toggle = () => {
    openForSession = !open;
    setOpen(openForSession);
  };

  return (
    <div data-analysis-log-footer className="flex flex-col gap-2 pt-2" style={{ borderTop: "1px solid var(--ab-border)" }}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls="analysis-log-body"
        onClick={toggle}
        className="self-start flex items-center gap-1 text-[10px] px-2 py-1 rounded text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/70 transition-colors"
        style={{ border: "1px solid rgba(63,63,70,0.5)" }}
      >
        {open ? <ChevronDown size={11} aria-hidden /> : <ChevronRight size={11} aria-hidden />}
        Measurement log ({count})
      </button>
      <div id="analysis-log-body" hidden={!open}>
        {children}
      </div>
    </div>
  );
}

export default memo(AnalysisLogFooter);
