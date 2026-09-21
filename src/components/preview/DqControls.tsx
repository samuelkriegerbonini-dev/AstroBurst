import { useState, useRef, useEffect, memo } from "react";
import { AlertTriangle, ChevronDown, Loader2 } from "lucide-react";
import { useDqContext } from "../../context/PreviewContext";
import { hasBit, toggleBit } from "../../utils/dqFlags";

const BTN_CLASS = "flex items-center gap-1 text-[10px] px-2 py-0.5 rounded transition-all duration-200 disabled:opacity-30 disabled:cursor-not-allowed";
const ON_STYLE: React.CSSProperties = { background: "rgba(248,113,113,0.15)", color: "#f87171", border: "1px solid rgba(248,113,113,0.35)" };
const OFF_STYLE: React.CSSProperties = { color: "#71717a", border: "1px solid transparent" };

function DqControlsInner() {
  const { plane, flagTable, overlay, setOverlay, excludeDq, setExcludeDq, dqMaskLoading, dqMaskError } = useDqContext();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const hasDq = !!plane?.dq_ref;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  useEffect(() => {
    if (!hasDq) setOpen(false);
  }, [hasDq]);

  const toggleTitle = !hasDq
    ? "No DQ plane in this file"
    : dqMaskError
      ? `DQ overlay failed: ${dqMaskError}`
      : overlay.enabled
        ? "DQ overlay shown — click to hide"
        : "Show flagged DQ pixels over the image";

  return (
    <div ref={rootRef} className="relative flex items-center gap-0.5">
      <button
        onClick={() => { if (hasDq) setOverlay({ enabled: !overlay.enabled }); }}
        disabled={!hasDq}
        title={toggleTitle}
        className={BTN_CLASS}
        style={overlay.enabled ? ON_STYLE : OFF_STYLE}
      >
        {dqMaskLoading ? <Loader2 size={10} className="animate-spin" /> : <AlertTriangle size={10} />}
        DQ
      </button>
      <button
        onClick={() => setOpen((v) => !v)}
        disabled={!hasDq}
        title={hasDq ? "DQ flags and masking options" : "No DQ plane in this file"}
        className="p-0.5 rounded text-zinc-500 hover:text-zinc-300 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
      >
        <ChevronDown size={10} />
      </button>

      {open && hasDq && (
        <div
          className="absolute right-0 top-full mt-1 z-50 w-60 rounded-md shadow-lg flex flex-col"
          style={{ background: "rgba(9,9,20,0.97)", border: "1px solid var(--ab-border)" }}
        >
          <div className="flex items-center justify-between px-2.5 py-1.5" style={{ borderBottom: "1px solid var(--ab-border)" }}>
            <span className="text-[10px] font-semibold text-zinc-300 uppercase tracking-wider">DQ flags</span>
            {flagTable && <span className="text-[9px] font-mono text-zinc-500 truncate max-w-[120px]" title={flagTable.label}>{flagTable.label}</span>}
          </div>

          {dqMaskError && (
            <div className="px-2.5 py-1.5 text-[9px] text-red-400 break-words" style={{ borderBottom: "1px solid var(--ab-border)" }}>
              overlay unavailable: {dqMaskError}
            </div>
          )}

          {!flagTable ? (
            <div className="flex items-center justify-center py-3">
              <Loader2 size={12} className="animate-spin text-zinc-600" />
            </div>
          ) : (
            <>
              <div className="flex items-center gap-2 px-2.5 py-1 text-[9px]">
                <button
                  onClick={() => setOverlay({ mask: flagTable.default_mask })}
                  className="text-zinc-400 hover:text-zinc-200 transition-colors"
                >
                  default
                </button>
                <button
                  onClick={() => setOverlay({ mask: 0 })}
                  className="text-zinc-400 hover:text-zinc-200 transition-colors"
                >
                  none
                </button>
                <span className="ml-auto font-mono text-zinc-600">mask {overlay.mask >>> 0}</span>
              </div>
              <div className="max-h-[220px] overflow-y-auto px-1.5 pb-1">
                {flagTable.flags.map((f) => (
                  <label
                    key={f.bit}
                    className="flex items-center gap-2 px-1 py-[2px] rounded cursor-pointer hover:bg-zinc-800/40"
                  >
                    <input
                      type="checkbox"
                      className="accent-red-400"
                      checked={hasBit(overlay.mask, f.bit)}
                      onChange={() => setOverlay({ mask: toggleBit(overlay.mask, f.bit) })}
                    />
                    <span className="text-[10px] font-mono text-zinc-300 truncate">{f.name}</span>
                    <span className="ml-auto text-[9px] font-mono text-zinc-600">{f.bit}</span>
                  </label>
                ))}
              </div>
            </>
          )}

          <label
            className="flex items-center gap-2 px-2.5 py-1.5 cursor-pointer"
            style={{ borderTop: "1px solid var(--ab-border)" }}
            title="Pixels flagged by the exclusion mask are ignored by histogram, stats and photometry"
          >
            <input
              type="checkbox"
              className="accent-red-400"
              checked={excludeDq}
              onChange={(e) => setExcludeDq(e.target.checked)}
            />
            <span className="text-[10px] text-zinc-300">exclude DO_NOT_USE from stats</span>
          </label>
        </div>
      )}
    </div>
  );
}

export default memo(DqControlsInner);
