import { useState, useEffect, memo } from "react";
import { Globe } from "lucide-react";
import { getWcsInfo, pixelToWorld } from "../../services/astrometry";
import type { WcsInfo } from "../../shared/types/astrometry";

interface WcsReadoutProps {
  filePath: string | null;
  imageWidth: number;
  imageHeight: number;
  mouseX: number | null;
  mouseY: number | null;
}

interface CelestialCoord {
  ra: number;
  dec: number;
}

function formatRA(ra: number): string {
  const h = ra / 15;
  const hours = Math.floor(h);
  const minutes = Math.floor((h - hours) * 60);
  const seconds = ((h - hours) * 60 - minutes) * 60;
  return `${hours}h ${minutes}m ${seconds.toFixed(2)}s`;
}

function formatDec(dec: number): string {
  const sign = dec >= 0 ? "+" : "-";
  const abs = Math.abs(dec);
  const degrees = Math.floor(abs);
  const arcmin = Math.floor((abs - degrees) * 60);
  const arcsec = ((abs - degrees) * 60 - arcmin) * 60;
  return `${sign}${degrees}° ${arcmin}' ${arcsec.toFixed(1)}"`;
}

// The mouse-pixel store already throttles to one distinct-integer-pixel update
// per animation frame; this debounce additionally caps how often a fast drag
// round-trips to the WCS engine over IPC (the readout used to do this pix->sky
// math synchronously in-process via src/utils/wcstransform.ts -- now retired in
// favor of pixel_to_world_cmd, which gets the full wcs-rs projection coverage).
const HOVER_DEBOUNCE_MS = 40;

function WcsReadoutInner({ filePath, mouseX, mouseY }: WcsReadoutProps) {
  const [wcsAvailable, setWcsAvailable] = useState<boolean | null>(null);
  const [wcsInfo, setWcsInfo] = useState<WcsInfo | null>(null);
  const [coord, setCoord] = useState<CelestialCoord | null>(null);

  useEffect(() => {
    if (!filePath) {
      setWcsAvailable(null);
      setWcsInfo(null);
      setCoord(null);
      return;
    }
    let cancelled = false;
    getWcsInfo(filePath)
      .then((info) => {
        if (cancelled) return;
        setWcsAvailable(true);
        setWcsInfo(info);
      })
      .catch(() => {
        if (cancelled) return;
        setWcsAvailable(false);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  useEffect(() => {
    if (!filePath || !wcsAvailable || mouseX === null || mouseY === null) {
      setCoord(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      pixelToWorld(filePath, [[mouseX, mouseY]])
        .then((res) => {
          if (cancelled) return;
          const pt = res.points[0];
          setCoord(pt ? { ra: pt[0], dec: pt[1] } : null);
        })
        .catch(() => {
          if (cancelled) return;
          setCoord(null);
        });
    }, HOVER_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [filePath, wcsAvailable, mouseX, mouseY]);

  if (!wcsAvailable || !wcsInfo) return null;

  return (
    <div
      className="flex items-center gap-3 text-[10px] font-mono"
      style={{ color: "rgba(52,211,153,0.6)" }}
    >
      <Globe size={10} />
      {wcsInfo.pixel_scale_arcsec && (
        <span>{wcsInfo.pixel_scale_arcsec.toFixed(2)}"/px</span>
      )}
      {coord ? (
        <>
          <span>RA {formatRA(coord.ra)}</span>
          <span>Dec {formatDec(coord.dec)}</span>
        </>
      ) : wcsInfo.center_ra !== undefined ? (
        <>
          <span>RA {formatRA(wcsInfo.center_ra)}</span>
          <span>Dec {formatDec(wcsInfo.center_dec)}</span>
        </>
      ) : null}
      {mouseX !== null && mouseY !== null && (
        <span className="text-zinc-600">
          px({mouseX},{mouseY})
        </span>
      )}
    </div>
  );
}

export default memo(WcsReadoutInner);
