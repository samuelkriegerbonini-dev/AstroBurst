import { Crosshair, Telescope } from "lucide-react";
import type { ArchiveController } from "./state/archiveController";
import { SUGGESTIONS, FIRST_RUN_HINTS } from "./logic/suggestions";
import { aroundLabel, type AroundTarget } from "./logic/aroundQuery";

interface FirstRunProps {
  ctl: Pick<ArchiveController, "runSuggestion" | "searchAround" | "openRomanAccess">;
  around: AroundTarget | null;
  onSwitchToWorkspace?: () => void;
}

export default function FirstRun({ ctl, around, onSwitchToWorkspace }: FirstRunProps) {
  const modifier = typeof navigator !== "undefined" && navigator.platform?.includes("Mac") ? "⌘" : "Ctrl+";
  return (
    <div className="ab-mast-hero">
      <Telescope size={28} aria-hidden="true" className="ab-mast-hero-icon" />
      <h2 className="ab-mast-hero-title">Explore the sky through Webb and Hubble</h2>
      <p className="ab-mast-hero-text">
        Search MAST by target, coordinates, programme or dataset id. Previews are quicklook images made by MAST; no science files are downloaded.
      </p>
      <div className="ab-mast-suggestions">
        <span className="ab-mast-hero-label">Try</span>
        {SUGGESTIONS.map((s) => (
          <button key={s.id} type="button" data-testid="mast-suggestion" data-query={s.text} data-mission={s.mission} className="ab-mast-chip ab-mast-suggestion" onClick={() => void ctl.runSuggestion(s)}>
            {s.label}
          </button>
        ))}
      </div>
      {around ? (
        <div className="ab-mast-suggestions">
          <button
            type="button"
            data-testid="mast-around"
            data-ra={around.raDeg}
            data-dec={around.decDeg}
            data-radius={around.radiusArcmin}
            className="ab-mast-chip ab-mast-suggestion"
            title="Search around the centre of the selected workspace image"
            onClick={() => void ctl.searchAround(around)}
          >
            <Crosshair size={11} aria-hidden="true" />
            {aroundLabel(around)}
          </button>
        </div>
      ) : null}
      <p className="ab-mast-hero-note">
        JWST, HST, IUE, CLASSY and ULLYSES need no account. Roman needs a MAST token and access granted by STScI.{" "}
        <button type="button" className="ab-mast-link-btn" onClick={() => ctl.openRomanAccess()}>
          About Roman access
        </button>
      </p>
      <p className="ab-mast-hero-note">A query MAST has not seen recently can take a minute; repeats take 1 to 3 s.</p>
      <div className="ab-mast-hints">
        {FIRST_RUN_HINTS.map((h) => (
          <span key={h.keys} className="ab-mast-hint">
            <kbd className="ab-cmdp-kbd">{h.keys}</kbd>
            {h.desc}
          </span>
        ))}
        {onSwitchToWorkspace ? (
          <button type="button" className="ab-mast-link-btn" onClick={onSwitchToWorkspace}>
            Workspace
            <kbd className="ab-cmdp-kbd">{`${modifier}1`}</kbd>
          </button>
        ) : null}
      </div>
    </div>
  );
}
