import { memo } from "react";
import ResolutionCard from "./ResolutionCard";
import SedPanel from "./SedPanel";
import { forcedSedPoints } from "../../utils/forcedPhotometry";
import { sizeSource } from "../../utils/forcedPhotometryRequest";
import { significant } from "../../utils/resolutionText";
import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";
import { sedNotes, sourceSizeBand, sourceTitle } from "./forcedSourceDetailModel";

export interface ForcedSourceDetailProps {
  result: ForcedPhotometryResult;
  source: number;
  currentPath?: string | null;
}

export function ForcedSourceDetailInner({ result, source, currentPath }: ForcedSourceDetailProps) {
  const shown = result.sources[source];
  if (shown === undefined) return null;
  const sed = forcedSedPoints(result, source);
  const label = `${shown.id}, ${sed.basis === "total" ? "total" : "aperture"} fluxes`;
  const fileNames = result.bands.filter((band) => band.error === null).map((band) => band.file_name);
  const sizeIndex = sourceSizeBand(result, source, currentPath ?? null);
  const sizeBandInfo = sizeIndex === null ? null : result.bands[sizeIndex];
  const size = sizeIndex === null ? null : sizeSource(shown.cells[sizeIndex] ?? null, result.bands[sizeIndex]);
  return (
    <div data-forced-source={source} className="space-y-1">
      <div data-forced-source-title className="text-[10px] font-mono text-zinc-300 break-all">
        {sourceTitle(result, source)}
      </div>
      <SedPanel points={sed.points} unit="uJy" label={label} fileNames={fileNames} />
      {sedNotes(result, source).map((note) => (
        <div key={note} data-forced-sed-note className="text-[9px] text-zinc-500">
          {note}
        </div>
      ))}
      {sizeBandInfo === null || size === null ? (
        <div data-forced-no-size className="text-[9px] text-zinc-500">
          No size for this source: its FWHM could not be measured at the forced position in any band. That width is a fit to the
          core of a compact source: it finds none on a source too faint for it or far wider than the PSF (a large galaxy, a
          blend), so this is not an unresolved verdict. To size an extended source, draw an ellipse on it and read R50 against
          the PSF in the Image tool, section Profiles.
        </div>
      ) : (
        <div data-forced-size-card={sizeBandInfo.index} className="space-y-1">
          <div className="text-[9px] text-zinc-400">
            {`Size measured in ${sizeBandInfo.label}: FWHM ${significant(size.fwhmArcsec)} arcsec at the forced position.`}
          </div>
          <ResolutionCard
            filePath={sizeBandInfo.path}
            fwhmArcsec={size.fwhmArcsec}
            snr={size.snr}
            context="forced"
            sourceLabel={`${shown.id} in ${sizeBandInfo.label} (${sizeBandInfo.file_name})`}
          />
        </div>
      )}
    </div>
  );
}

export default memo(ForcedSourceDetailInner);
