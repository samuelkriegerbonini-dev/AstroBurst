import { useState, useEffect, useMemo, memo } from "react";
import { Globe } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import { pixelToWorld, type WcsParams, type CelestialCoord } from "../../utils/wcstransform";

interface WcsReadoutProps {
  filePath: string | null;
  imageWidth: number;
  imageHeight: number;
  mouseX: number | null;
  mouseY: number | null;
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

function WcsReadoutInner({ filePath, imageWidth, imageHeight, mouseX, mouseY }: WcsReadoutProps) {
  const { getWcsInfo } = useBackend();
  const [wcsAvailable, setWcsAvailable] = useState<boolean | null>(null);
  const [wcsInfo, setWcsInfo] = useState<any>(null);
  const [wcsParams, setWcsParams] = useState<WcsParams | null>(null);

  useEffect(() => {
    if (!filePath) {
      setWcsAvailable(null);
      setWcsInfo(null);
      setWcsParams(null);
      return;
    }
    let cancelled = false;
    getWcsInfo(filePath)
      .then((info: any) => {
        if (cancelled) return;
        setWcsAvailable(true);
        setWcsInfo(info);
        if (info.wcs_params) {
          setWcsParams(info.wcs_params as WcsParams);
        }
      })
      .catch(() => {
        if (cancelled) return;
        setWcsAvailable(false);
      });
    return () => { cancelled = true; };
  }, [filePath, getWcsInfo]);

  const coord: CelestialCoord | null = useMemo(() => {
    if (!wcsParams || mouseX === null || mouseY === null) return null;
    return pixelToWorld(wcsParams, mouseX, mouseY);
  }, [wcsParams, mouseX, mouseY]);

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
