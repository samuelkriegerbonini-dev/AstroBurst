import { memo, useEffect, useRef, useState } from "react";
import DecimalField from "../ui/DecimalField";
import ErrorAlert from "../ui/ErrorAlert";
import WarningList from "../ui/WarningList";
import { useFileContext } from "../../context/PreviewContext";
import { applyLayerChange } from "../../hooks/applyLayerChange";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { measurementLog } from "../../utils/measurementLog";
import { layerAssignmentEntry, type LayerChange } from "../../utils/measurementLogLayers";
import { clearCompanion, setCompanion, setSdssFrameError } from "../../services/companions";
import { decimalDraftText, decimalFieldState } from "../../utils/decimalText";
import { ERROR_KIND_OPTIONS, fileLayerName, handSet, layerLines, shownLayerText, type LayerLine, type PendingLayer } from "../../utils/layerLabels";
import type { PlaneInfo } from "../../shared/types/fits.types";
import type { CompanionRole, ErrorLayerRole } from "../../shared/types/layers";

interface CompanionAssignProps {
  imagePath: string;
  plane: PlaneInfo;
  pending: PendingLayer | null;
  onPendingChange: (pending: PendingLayer | null) => void;
}

type ErrorKindChoice = ErrorLayerRole | "auto";

const FITS_FILTERS = [{ name: "FITS", extensions: ["fits", "fit", "fts", "fz", "gz", "bz2"] }];
const DIALOG_TITLES: Record<CompanionRole, string> = { err: "Error layer of the shown image", dq: "Mask of the shown image" };
const PENDING_TARGETS: Record<CompanionRole, string> = { err: "the error layer", dq: "the mask" };
const HAND_SET_NOTE = "Layers set by hand apply to new measurements. The time series and the log keep what they measured before the change.";
const GAIN_LABEL = "Gain (e-/DN)";
const DARK_LABEL = "Dark variance (DN^2)";
const GAIN_ERROR = "Use a value above 0";
const DARK_MIN = 0;

const BUTTON_CLASS =
  "text-[9px] font-medium px-1.5 py-px rounded transition-colors border border-zinc-600/40 text-zinc-400 hover:bg-zinc-700/40 disabled:opacity-40 disabled:cursor-not-allowed";
const FIELD_CLASS = "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-[10px] text-zinc-200 font-mono focus:border-teal-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const FIELD_LABEL_CLASS = "flex flex-col gap-0.5 text-[9px] text-zinc-400";
const MUTED_TEXT_CLASS = "text-[10px] text-[var(--ab-text-3)]";
const LINE_TEXT_CLASS: Record<LayerLine["state"], string> = {
  in_use: "text-zinc-300",
  not_used: "text-amber-300",
  none: "text-[var(--ab-text-3)]",
};
const ORIGIN_CHIP_STYLE = { background: "rgba(20,184,166,0.12)", border: "1px solid rgba(20,184,166,0.3)", color: "var(--ab-teal)" };
const PENDING_STYLE = { background: "rgba(20,184,166,0.05)", border: "1px solid rgba(20,184,166,0.2)" };

const lineText = (line: LayerLine) => (line.state === "not_used" ? `not used: ${line.text}` : line.text);
const messageOf = (e: unknown) => (e instanceof Error ? e.message : String(e));

function CompanionAssign({ imagePath, plane, pending, onPendingChange }: CompanionAssignProps) {
  const { file } = useFileContext();
  const provenance = useMeasurementProvenance();
  const recipe = plane.err_recipe ?? null;
  const recipeGain = recipe?.gain ?? null;
  const recipeDark = recipe?.dark_variance ?? null;
  const [kind, setKind] = useState<ErrorKindChoice>("auto");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gainText, setGainText] = useState(() => decimalDraftText(recipeGain));
  const [darkText, setDarkText] = useState(() => decimalDraftText(recipeDark));
  const shownPath = useRef(imagePath);

  useEffect(() => {
    setKind("auto");
    setError(null);
  }, [pending]);

  useEffect(() => {
    shownPath.current = imagePath;
    setBusy(false);
    setError(null);
    setGainText(decimalDraftText(recipeGain));
    setDarkText(decimalDraftText(recipeDark));
  }, [imagePath, recipeGain, recipeDark]);

  const actor = file !== null && file.result !== null && file.path === imagePath ? file : null;
  const disabled = actor === null || busy;
  const gain = decimalFieldState(gainText).value;
  const dark = decimalFieldState(darkText, { min: DARK_MIN }).value;
  const noise = gain !== null && gain > 0 && dark !== null ? { gain, dark } : null;
  const shownText = shownLayerText(plane);

  const run = (change: LayerChange, request: () => Promise<PlaneInfo>) => {
    if (actor === null) return;
    const requestedFor = imagePath;
    setBusy(true);
    setError(null);
    return request().then(
      (changed) => {
        if (shownPath.current !== requestedFor) return;
        applyLayerChange(actor, changed);
        measurementLog.append(layerAssignmentEntry(provenance, change, changed));
        onPendingChange(null);
        setBusy(false);
      },
      (e: unknown) => {
        if (shownPath.current !== requestedFor) return;
        setError(messageOf(e));
        setBusy(false);
      },
    );
  };

  const assign = (layer: PendingLayer) =>
    run({ role: layer.role, action: "assign" }, () =>
      setCompanion(imagePath, layer.role, layer.source, layer.role === "err" && kind !== "auto" ? kind : null),
    );

  const chooseFile = async (role: CompanionRole) => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: false, title: DIALOG_TITLES[role], filters: FITS_FILTERS });
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (path) onPendingChange({ role, source: path, name: fileLayerName(path) });
  };

  return (
    <section aria-label="Layers of the shown image" className="flex flex-col gap-2 px-3 py-2" style={{ borderBottom: "1px solid var(--ab-border)" }}>
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-zinc-400">Layers of the shown image</h3>
      {shownText !== null ? (
        <p className="text-[10px] text-zinc-300 break-words select-text">{shownText}</p>
      ) : (
        <>
          {layerLines(plane).map((line) => (
            <div key={line.role} role="group" aria-label={line.title} className="flex flex-col gap-1">
              <div className="flex items-start gap-2">
                <span className="w-[34px] shrink-0 text-[10px] font-medium text-zinc-400">{line.title}</span>
                <span className={`flex-1 min-w-0 break-words select-text text-[10px] ${LINE_TEXT_CLASS[line.state]}`} title={lineText(line)}>
                  {lineText(line)}
                </span>
                {line.chip && (
                  <span className="shrink-0 text-[9px] font-medium px-1.5 py-px rounded" style={ORIGIN_CHIP_STYLE}>
                    {line.chip}
                  </span>
                )}
              </div>
              {line.detail && <span className={`pl-[42px] ${MUTED_TEXT_CLASS}`}>{line.detail}</span>}
              <div className="flex flex-wrap gap-1 pl-[42px]">
                <button type="button" className={BUTTON_CLASS} disabled={disabled} onClick={() => chooseFile(line.role)}>
                  Choose a file
                </button>
                {line.canSwitchOff && (
                  <button type="button" className={BUTTON_CLASS} disabled={disabled} onClick={() => run({ role: line.role, action: "switch_off" }, () => clearCompanion(imagePath, line.role, true))}>
                    Switch off
                  </button>
                )}
                {line.canAutomatic && (
                  <button type="button" className={BUTTON_CLASS} disabled={disabled} onClick={() => run({ role: line.role, action: "automatic" }, () => clearCompanion(imagePath, line.role, false))}>
                    Automatic
                  </button>
                )}
              </div>
            </div>
          ))}
          {pending && (
            <div className="flex flex-col gap-1.5 rounded px-2 py-1.5" style={PENDING_STYLE}>
              <span className="text-[10px] text-zinc-300 break-words select-text">{`Use ${pending.name} as ${PENDING_TARGETS[pending.role]}`}</span>
              {pending.role === "err" && (
                <select aria-label="Error layer kind" value={kind} onChange={(e) => setKind(e.target.value as ErrorKindChoice)} className={SELECT_CLASS}>
                  {ERROR_KIND_OPTIONS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              )}
              <div className="flex gap-1">
                <button type="button" className={BUTTON_CLASS} disabled={disabled} onClick={() => assign(pending)}>
                  Assign
                </button>
                <button type="button" className={BUTTON_CLASS} disabled={disabled} onClick={() => onPendingChange(null)}>
                  Cancel
                </button>
              </div>
            </div>
          )}
          {recipe?.kind === "sdss_frame" && (
            <div className="flex flex-col gap-1.5 pt-2" style={{ borderTop: "1px solid var(--ab-border)" }}>
              <h4 className="text-[10px] font-semibold text-zinc-300">SDSS frame error</h4>
              <p className={MUTED_TEXT_CLASS}>Neither number is in the file. Type the gain and the dark variance of this camera column and band.</p>
              <div className="grid grid-cols-2 gap-2">
                <label className={FIELD_LABEL_CLASS}>
                  {GAIN_LABEL}
                  <DecimalField
                    aria-label={GAIN_LABEL}
                    value={gainText}
                    onChange={setGainText}
                    error={gain !== null && gain <= 0 ? GAIN_ERROR : null}
                    showError
                    className={FIELD_CLASS}
                  />
                </label>
                <label className={FIELD_LABEL_CLASS}>
                  {DARK_LABEL}
                  <DecimalField aria-label={DARK_LABEL} value={darkText} onChange={setDarkText} min={DARK_MIN} showError className={FIELD_CLASS} />
                </label>
              </div>
              <button
                type="button"
                className={`${BUTTON_CLASS} self-start`}
                disabled={disabled || noise === null}
                onClick={() => noise && run({ role: "err", action: "sdss_recipe" }, () => setSdssFrameError(imagePath, noise.gain, noise.dark))}
              >
                Rebuild the error layer
              </button>
            </div>
          )}
        </>
      )}
      <ErrorAlert message={error} compact />
      {(plane.pairing_notes ?? []).map((note, index) => (
        <p key={index} className={`${MUTED_TEXT_CLASS} break-words select-text`}>
          {note}
        </p>
      ))}
      <WarningList warnings={plane.warnings} />
      {handSet(plane) && <p className={MUTED_TEXT_CLASS}>{HAND_SET_NOTE}</p>}
    </section>
  );
}

export default memo(CompanionAssign);
