import { memo } from "react";
import { RotateCcw } from "lucide-react";
import { useDisplayContext } from "../../context/PreviewContext";
import { normalizePercentiles, normalizeUserLimits } from "../../utils/displayLimits";
import {
  COLORMAP_NAMES,
  DEFAULT_DISPLAY_SETTINGS,
  GRID_DENSITIES,
  GRID_FRAMES,
  LIMIT_MODES,
  STRETCH_MODES,
  type ColormapName,
  type GridFrame,
  type LimitMode,
  type StretchMode,
} from "../../shared/types/display";

interface DisplayControlsProps {
  vmin: number;
  vmax: number;
}

const SELECT_CLASS =
  "bg-zinc-900/80 border border-zinc-700/60 rounded px-1 py-0.5 text-[10px] text-zinc-200 focus:outline-none focus:border-zinc-500";
const INPUT_CLASS =
  "bg-zinc-900/80 border border-zinc-700/60 rounded px-1 py-0.5 text-[10px] font-mono text-zinc-200 w-16 focus:outline-none focus:border-zinc-500";
const LABEL_CLASS = "text-[9px] uppercase tracking-wide text-zinc-500";

function formatLimit(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return v.toExponential(3);
  return v.toFixed(abs < 10 ? 4 : 2);
}

function parseNullable(text: string): number | null | undefined {
  if (text.trim() === "") return null;
  const n = Number(text);
  return Number.isFinite(n) ? n : undefined;
}

function DisplayControlsInner({ vmin, vmax }: DisplayControlsProps) {
  const { display, setDisplay, limitsLoading, limitsError } = useDisplayContext();
  const isMtf = display.stretch === "mtf";

  const commitPercentiles = () => {
    const [lo, hi] = normalizePercentiles(display.percentileLow, display.percentileHigh);
    if (lo !== display.percentileLow || hi !== display.percentileHigh) {
      setDisplay({ percentileLow: lo, percentileHigh: hi });
    }
  };

  const commitUserLimits = () => {
    const [lo, hi] = normalizeUserLimits(display.userLo, display.userHi);
    if (lo !== display.userLo || hi !== display.userHi) {
      setDisplay({ userLo: lo, userHi: hi });
    }
  };

  const onCommitKey = (commit: () => void) => (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    commit();
    e.currentTarget.blur();
  };

  const onNumber = (key: "percentileLow" | "percentileHigh" | "zscaleContrast" | "asinhA" | "power") =>
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const n = Number(e.target.value);
      if (e.target.value.trim() === "" || !Number.isFinite(n)) return;
      setDisplay({ [key]: n });
    };

  const onNullable = (key: "userLo" | "userHi") => (e: React.ChangeEvent<HTMLInputElement>) => {
    const parsed = parseNullable(e.target.value);
    if (parsed === undefined) return;
    setDisplay({ [key]: parsed });
  };

  return (
    <div
      className="flex items-center gap-2 px-3 py-1 border-b border-zinc-800/80 flex-wrap"
      style={{ background: "rgba(24,24,27,0.6)" }}
    >
      <label className="flex items-center gap-1" title="Stretch curve applied after normalisation">
        <span className={LABEL_CLASS}>stretch</span>
        <select
          className={SELECT_CLASS}
          value={display.stretch}
          onChange={(e) => setDisplay({ stretch: e.target.value as StretchMode })}
        >
          {STRETCH_MODES.map((s) => (
            <option key={s} value={s}>{s}</option>
          ))}
        </select>
      </label>

      {display.stretch === "asinh" && (
        <label className="flex items-center gap-1" title="asinh softening parameter a">
          <span className={LABEL_CLASS}>a</span>
          <input
            type="number"
            step={0.01}
            min={0.0001}
            className={INPUT_CLASS}
            value={display.asinhA}
            onChange={onNumber("asinhA")}
          />
        </label>
      )}

      {display.stretch === "power" && (
        <label className="flex items-center gap-1" title="power-law exponent">
          <span className={LABEL_CLASS}>p</span>
          <input
            type="number"
            step={0.1}
            min={0.01}
            className={INPUT_CLASS}
            value={display.power}
            onChange={onNumber("power")}
          />
        </label>
      )}

      <label
        className={`flex items-center gap-1 ${isMtf ? "opacity-40" : ""}`}
        title={isMtf ? "Limits are the data min/max while stretch is mtf" : "How vmin/vmax are chosen"}
      >
        <span className={LABEL_CLASS}>limits</span>
        <select
          className={SELECT_CLASS}
          value={display.limits}
          disabled={isMtf}
          onChange={(e) => setDisplay({ limits: e.target.value as LimitMode })}
        >
          {LIMIT_MODES.map((l) => (
            <option key={l} value={l}>{l}</option>
          ))}
        </select>
      </label>

      {!isMtf && display.limits === "percentile" && (
        <>
          <label className="flex items-center gap-1" title="lower percentile (%)">
            <span className={LABEL_CLASS}>lo%</span>
            <input
              type="number"
              step={0.1}
              min={0}
              max={100}
              className={INPUT_CLASS}
              value={display.percentileLow}
              onChange={onNumber("percentileLow")}
              onBlur={commitPercentiles}
              onKeyDown={onCommitKey(commitPercentiles)}
            />
          </label>
          <label className="flex items-center gap-1" title="upper percentile (%)">
            <span className={LABEL_CLASS}>hi%</span>
            <input
              type="number"
              step={0.1}
              min={0}
              max={100}
              className={INPUT_CLASS}
              value={display.percentileHigh}
              onChange={onNumber("percentileHigh")}
              onBlur={commitPercentiles}
              onKeyDown={onCommitKey(commitPercentiles)}
            />
          </label>
        </>
      )}

      {!isMtf && display.limits === "user" && (
        <>
          <label className="flex items-center gap-1" title="vmin (empty = data min)">
            <span className={LABEL_CLASS}>lo</span>
            <input
              type="number"
              step="any"
              className={INPUT_CLASS}
              value={display.userLo ?? ""}
              placeholder="min"
              onChange={onNullable("userLo")}
              onBlur={commitUserLimits}
              onKeyDown={onCommitKey(commitUserLimits)}
            />
          </label>
          <label className="flex items-center gap-1" title="vmax (empty = data max)">
            <span className={LABEL_CLASS}>hi</span>
            <input
              type="number"
              step="any"
              className={INPUT_CLASS}
              value={display.userHi ?? ""}
              placeholder="max"
              onChange={onNullable("userHi")}
              onBlur={commitUserLimits}
              onKeyDown={onCommitKey(commitUserLimits)}
            />
          </label>
        </>
      )}

      {!isMtf && display.limits === "zscale" && (
        <label className="flex items-center gap-1" title="zscale contrast">
          <span className={LABEL_CLASS}>contrast</span>
          <input
            type="number"
            step={0.05}
            min={0.01}
            max={1}
            className={INPUT_CLASS}
            value={display.zscaleContrast}
            onChange={onNumber("zscaleContrast")}
          />
        </label>
      )}

      <label className="flex items-center gap-1" title="colour lookup table">
        <span className={LABEL_CLASS}>cmap</span>
        <select
          className={SELECT_CLASS}
          value={display.colormap}
          onChange={(e) => setDisplay({ colormap: e.target.value as ColormapName })}
        >
          {COLORMAP_NAMES.map((c) => (
            <option key={c} value={c}>{c}</option>
          ))}
        </select>
      </label>

      <label className="flex items-center gap-1 cursor-pointer" title="invert the lookup table">
        <input
          type="checkbox"
          className="accent-zinc-400"
          checked={display.invert}
          onChange={(e) => setDisplay({ invert: e.target.checked })}
        />
        <span className={LABEL_CLASS}>invert</span>
      </label>

      <label className="flex items-center gap-1 cursor-pointer" title="WCS coordinate grid overlay (needs a plate-solved image)">
        <input
          type="checkbox"
          className="accent-zinc-400"
          checked={display.grid}
          onChange={(e) => setDisplay({ grid: e.target.checked })}
        />
        <span className={LABEL_CLASS}>grid</span>
      </label>

      {display.grid && (
        <>
          <label className="flex items-center gap-1" title="sky frame of the coordinate grid">
            <span className={LABEL_CLASS}>frame</span>
            <select
              className={SELECT_CLASS}
              value={display.gridFrame}
              onChange={(e) => setDisplay({ gridFrame: e.target.value as GridFrame })}
            >
              {GRID_FRAMES.map((f) => (
                <option key={f} value={f}>{f}</option>
              ))}
            </select>
          </label>
          <label className="flex items-center gap-1" title="grid density (1 = sparse, 5 = dense)">
            <span className={LABEL_CLASS}>density</span>
            <select
              className={SELECT_CLASS}
              value={display.gridDensity}
              onChange={(e) => setDisplay({ gridDensity: Number(e.target.value) })}
            >
              {GRID_DENSITIES.map((d) => (
                <option key={d} value={d}>{d}</option>
              ))}
            </select>
          </label>
        </>
      )}

      <span className="text-[9px] font-mono text-zinc-500 ml-auto" title="resolved display limits">
        {limitsLoading ? "…" : `${formatLimit(vmin)} … ${formatLimit(vmax)}`}
      </span>

      <button
        onClick={() => setDisplay(DEFAULT_DISPLAY_SETTINGS)}
        className="flex items-center gap-1 text-[9px] text-zinc-400 hover:text-zinc-200 transition-colors"
        title="Reset display settings"
      >
        <RotateCcw size={9} />
        Reset
      </button>

      {!isMtf && limitsError && (
        <span className="w-full text-[9px] text-red-400/80 truncate" title={limitsError}>
          {limitsError}
        </span>
      )}
    </div>
  );
}

export default memo(DisplayControlsInner);
