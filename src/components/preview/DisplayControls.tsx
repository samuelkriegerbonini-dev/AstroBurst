import { memo, useId, useSyncExternalStore } from "react";
import { Loader2, RotateCcw } from "lucide-react";
import "./display-controls.css";
import { useDisplayContext, useRawPixelsContext } from "../../context/PreviewContext";
import { normalizePercentiles, normalizeUserLimits } from "../../utils/displayLimits";
import { symmetricStretchNote } from "../../utils/displayTransfer";
import { wcsOverlayNote, wcsOverlayStatus } from "../../utils/wcsOverlayStatus";
import { NEEDS_GPU_TITLE } from "../../utils/previewShell";
import { linearTitle } from "../../utils/displayMemory";
import { DecimalNumberField } from "../ui";
import {
  COLORMAP_LABELS,
  COLORMAP_NAMES,
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
  renderOnlyDisabled?: boolean;
  renderOnlyTitle?: string;
}

const SELECT_CLASS = "ab-display-select";
const INPUT_CLASS = "ab-display-input";
const LABEL_CLASS = "ab-display-label";
const GROUP_CLASS = "ab-display-group";
const TOOL_BUTTON_CLASS = "ab-display-action";
const NOTE_CLASS = "ab-display-note";

const SAME_GROUP_TITLE =
  "Same display for frames of one instrument + filter: a file without its own display takes the last display set on a file with the same INSTRUME and filter (PUPIL when it names a filter). Off: such files start from the default display";
const SET_DEFAULT_TITLE =
  "Start files without their own display (and without a matching instrument + filter) from this stretch, limits, colormap, invert and symmetric setting; user lo/hi limits are not carried over";
const RESET_DISPLAY_TITLE =
  "Reset this file's display (stretch, limits, colormap, invert, symmetric, STF) and the colorbar, grid and compass";

function formatLimit(v: number): string {
  if (!Number.isFinite(v)) return "—";
  const abs = Math.abs(v);
  if (abs !== 0 && (abs < 1e-3 || abs >= 1e6)) return v.toExponential(3);
  return v.toFixed(abs < 10 ? 4 : 2);
}

function DisplayControlsInner({ vmin, vmax, renderOnlyDisabled = false, renderOnlyTitle = NEEDS_GPU_TITLE }: DisplayControlsProps) {
  const {
    display, setDisplay, limits, limitsLoading, limitsError,
    linear, linearBlocked, toggleLinear, resetDisplay, setAsDefault, sameGroup, setSameGroup,
  } = useDisplayContext();
  const { rawPixelsLoading } = useRawPixelsContext();
  const isMtf = display.stretch === "mtf";
  const mtfHintShown = isMtf && !renderOnlyDisabled;
  const mtfHintId = useId();
  const stretchNote = symmetricStretchNote(display);
  const wcsErrors = useSyncExternalStore(wcsOverlayStatus.subscribe, wcsOverlayStatus.get, wcsOverlayStatus.get);
  const wcsNote = wcsOverlayNote({ grid: display.grid, compass: display.compass }, wcsErrors);
  const wcsNoteId = useId();
  const gridNoteId = display.grid && wcsErrors.grid !== null ? wcsNoteId : undefined;
  const compassNoteId = display.compass && wcsErrors.compass !== null ? wcsNoteId : undefined;
  const renderTitle = (title: string) => (renderOnlyDisabled ? renderOnlyTitle : title);
  const renderGroupClass = `${GROUP_CLASS} ${renderOnlyDisabled ? "opacity-50" : ""}`;
  const renderGroupTitle = renderOnlyDisabled ? renderOnlyTitle : undefined;

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

  const onNumber = (key: "percentileLow" | "percentileHigh" | "zscaleContrast" | "asinhA" | "power" | "centre") =>
    (value: number | null) => {
      if (value !== null) setDisplay({ [key]: value });
    };

  const onNullable = (key: "userLo" | "userHi") => (value: number | null) => setDisplay({ [key]: value });

  return (
    <fieldset
      data-testid="display-controls"
      className="ab-display-controls"
      aria-label="Image display"
    >
      <div className={GROUP_CLASS}>
        <button
          type="button"
          data-testid="display-linear"
          aria-pressed={linear !== null}
          disabled={linearBlocked !== null}
          onClick={toggleLinear}
          title={linearBlocked ?? linearTitle(linear)}
          className="ab-display-toggle disabled:opacity-40"
        >
          Linear
        </button>
        {renderOnlyDisabled && (
          <span role="status" className={NOTE_CLASS}>
            {rawPixelsLoading && <Loader2 size={9} className="animate-spin" aria-hidden="true" />}
            {renderOnlyTitle}
          </span>
        )}
      </div>

      <fieldset disabled={renderOnlyDisabled} className={renderGroupClass} title={renderGroupTitle} style={{ minInlineSize: 0 }}>
        <label className="flex items-center gap-1" title={renderTitle("Stretch curve applied after normalisation")}>
          <span className={LABEL_CLASS}>stretch</span>
          <select
            className={SELECT_CLASS}
            aria-label="Stretch curve"
            value={display.stretch}
            onChange={(e) => setDisplay({ stretch: e.target.value as StretchMode })}
          >
            {STRETCH_MODES.map((s) => (
              <option key={s} value={s}>{s}</option>
            ))}
          </select>
        </label>

        {display.stretch === "asinh" && (
          <label className="flex items-center gap-1" title={renderTitle("asinh softening parameter a")}>
            <span className={LABEL_CLASS}>a</span>
            <DecimalNumberField
              min={0.0001}
              aria-label="Asinh softening parameter a"
              className={INPUT_CLASS}
              value={display.asinhA}
              onValueChange={onNumber("asinhA")}
            />
          </label>
        )}

        {display.stretch === "power" && (
          <label className="flex items-center gap-1" title={renderTitle("power-law exponent")}>
            <span className={LABEL_CLASS}>p</span>
            <DecimalNumberField
              min={0.01}
              aria-label="Power-law exponent p"
              className={INPUT_CLASS}
              value={display.power}
              onValueChange={onNumber("power")}
            />
          </label>
        )}
      </fieldset>

      <fieldset disabled={renderOnlyDisabled} className={renderGroupClass} title={renderGroupTitle} style={{ minInlineSize: 0 }}>
        <label
          className={`flex items-center gap-1 ${isMtf ? "opacity-40" : ""}`}
          title={renderTitle(isMtf ? "Limits are the data min/max while stretch is mtf" : "How vmin/vmax are chosen")}
        >
          <span className={LABEL_CLASS}>limits</span>
          <select
            className={SELECT_CLASS}
            aria-label="Display limits"
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
            <label className="flex items-center gap-1" title={renderTitle("lower percentile (%)")}>
              <span className={LABEL_CLASS}>lo%</span>
              <DecimalNumberField
                className={INPUT_CLASS}
                value={display.percentileLow}
                onValueChange={onNumber("percentileLow")}
                onBlur={commitPercentiles}
                onKeyDown={onCommitKey(commitPercentiles)}
              />
            </label>
            <label className="flex items-center gap-1" title={renderTitle("upper percentile (%)")}>
              <span className={LABEL_CLASS}>hi%</span>
              <DecimalNumberField
                className={INPUT_CLASS}
                value={display.percentileHigh}
                onValueChange={onNumber("percentileHigh")}
                onBlur={commitPercentiles}
                onKeyDown={onCommitKey(commitPercentiles)}
              />
            </label>
          </>
        )}

        {!isMtf && display.limits === "user" && (
          <>
            <label className="flex items-center gap-1" title={renderTitle("vmin (empty = data min)")}>
              <span className={LABEL_CLASS}>lo</span>
              <DecimalNumberField
                optional
                className={INPUT_CLASS}
                value={display.userLo}
                placeholder="min"
                onValueChange={onNullable("userLo")}
                onBlur={commitUserLimits}
                onKeyDown={onCommitKey(commitUserLimits)}
              />
            </label>
            <label className="flex items-center gap-1" title={renderTitle("vmax (empty = data max)")}>
              <span className={LABEL_CLASS}>hi</span>
              <DecimalNumberField
                optional
                className={INPUT_CLASS}
                value={display.userHi}
                placeholder="max"
                onValueChange={onNullable("userHi")}
                onBlur={commitUserLimits}
                onKeyDown={onCommitKey(commitUserLimits)}
              />
            </label>
          </>
        )}

        {!isMtf && display.limits === "zscale" && (
          <label className="flex items-center gap-1" title={renderTitle("zscale contrast")}>
            <span className={LABEL_CLASS}>contrast</span>
            <DecimalNumberField
              min={0.01}
              max={1}
              className={INPUT_CLASS}
              value={display.zscaleContrast}
              onValueChange={onNumber("zscaleContrast")}
            />
          </label>
        )}

        {!renderOnlyDisabled && (
          <span className="ab-display-range" title="resolved display limits">
            {limitsLoading
              ? "…"
              : `${formatLimit(vmin)} … ${formatLimit(vmax)}${
                  !isMtf && limits?.symmetric === true && limits.centre !== null
                    ? ` · c ${formatLimit(limits.centre)}`
                    : ""
                }`}
          </span>
        )}
      </fieldset>

      <fieldset disabled={renderOnlyDisabled} className={renderGroupClass} title={renderGroupTitle} style={{ minInlineSize: 0 }}>
        <label
          className="flex items-center gap-1"
          title={renderTitle("colour lookup table; RdBu, coolwarm, bwr and seismic are diverging: pair them with symmetric limits and a linear stretch")}
        >
          <span className={LABEL_CLASS}>cmap</span>
          <select
            className={SELECT_CLASS}
            aria-label="Color map"
            value={display.colormap}
            onChange={(e) => setDisplay({ colormap: e.target.value as ColormapName })}
          >
            {COLORMAP_NAMES.map((c) => (
              <option key={c} value={c}>{COLORMAP_LABELS[c]}</option>
            ))}
          </select>
        </label>
      </fieldset>

      <fieldset disabled={renderOnlyDisabled} className={renderGroupClass} title={renderGroupTitle} style={{ minInlineSize: 0 }}>
        <button
          type="button"
          data-testid="display-symmetric"
          aria-label="Symmetric"
          aria-pressed={display.symmetric}
          aria-describedby={mtfHintShown ? mtfHintId : undefined}
          disabled={renderOnlyDisabled}
          onClick={() => setDisplay({ symmetric: !display.symmetric })}
          className="ab-display-toggle"
          title={renderTitle("Mirror vmin/vmax about a centre: vmin = c - a, vmax = c + a with a = max(|lo - c|, |hi - c|) from the chosen limits; enabling it under mtf switches the stretch to linear")}
        >
          <span>symmetric</span>
        </button>

        {mtfHintShown && (
          <span id={mtfHintId} className="ab-display-hint">
            (sets stretch to linear)
          </span>
        )}

        {display.symmetric && (
          <label
            className="flex items-center gap-1"
            title={renderTitle("Centre c of the symmetric limits, in data units (default 0). Pixels that are exactly 0 are padding and take the no-data colour, not the centre colour; the centre lands on the colormap centre only under a linear stretch")}
          >
            <span className={LABEL_CLASS}>centre</span>
            <DecimalNumberField
              aria-label="Symmetric limits centre"
              className={INPUT_CLASS}
              value={display.centre}
              onValueChange={onNumber("centre")}
            />
          </label>
        )}

        {stretchNote && (
          <span className={NOTE_CLASS} title={stretchNote}>
            ({stretchNote})
          </span>
        )}

        <button
          type="button"
          data-testid="display-invert"
          aria-label="Invert"
          aria-pressed={display.invert}
          disabled={renderOnlyDisabled}
          onClick={() => setDisplay({ invert: !display.invert })}
          className="ab-display-toggle"
          title={renderTitle("invert the lookup table")}
        >
          <span>invert</span>
        </button>

        <button
          type="button"
          data-testid="display-colorbar"
          aria-label="Colorbar"
          aria-pressed={display.colorbar}
          disabled={renderOnlyDisabled}
          onClick={() => setDisplay({ colorbar: !display.colorbar })}
          className="ab-display-toggle"
          title={renderTitle("value scale under the image: ticks follow stretch, limits and colormap")}
        >
          <span>colorbar</span>
        </button>
      </fieldset>

      <div className={GROUP_CLASS}>
        <button
          type="button"
          data-testid="display-grid"
          aria-label="Grid"
          aria-pressed={display.grid}
          aria-describedby={gridNoteId}
          onClick={() => setDisplay({ grid: !display.grid })}
          className="ab-display-toggle"
          title="WCS coordinate grid overlay (needs a plate-solved image)"
        >
          <span>grid</span>
        </button>
        <button
          type="button"
          data-testid="display-compass"
          aria-label="Compass"
          aria-pressed={display.compass}
          aria-describedby={compassNoteId}
          onClick={() => setDisplay({ compass: !display.compass })}
          className="ab-display-toggle"
          title="north/east compass and scale bar overlay (needs a WCS)"
        >
          <span>compass</span>
        </button>

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

        {wcsNote && (
          <span id={wcsNoteId} className={NOTE_CLASS} title={wcsNote.title}>
            ({wcsNote.text})<span className="sr-only"> {wcsNote.title}</span>
          </span>
        )}
      </div>

      <div className="ab-display-memory">
        <button
          type="button"
          data-testid="display-same-group"
          aria-label="Same filter"
          aria-pressed={sameGroup}
          onClick={() => setSameGroup(!sameGroup)}
          className="ab-display-toggle"
          title={SAME_GROUP_TITLE}
        >
          <span>same filter</span>
        </button>
        <button type="button" data-testid="display-set-default" onClick={setAsDefault} className={TOOL_BUTTON_CLASS} title={SET_DEFAULT_TITLE}>
          Set as default
        </button>
        <button type="button" data-testid="display-reset" onClick={resetDisplay} className={TOOL_BUTTON_CLASS} title={RESET_DISPLAY_TITLE}>
          <RotateCcw size={9} />
          Reset display
        </button>
      </div>

      {!renderOnlyDisabled && !isMtf && limitsError && (
        <span className="w-full text-[9px] text-red-400/80 truncate" title={limitsError}>
          {limitsError}
        </span>
      )}
      {!renderOnlyDisabled && !isMtf && limits && limits.notes.length > 0 && (
        <span className="w-full text-[9px] text-amber-400/80 truncate" title={limits.notes.join("; ")}>
          {limits.notes.join("; ")}
        </span>
      )}
    </fieldset>
  );
}

export default memo(DisplayControlsInner);
