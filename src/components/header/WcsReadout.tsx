import { useState, useCallback, useRef, useEffect, memo } from "react";
import { Globe, Loader2 } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";

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
  const { pixelToWorld, getWcsInfo } = useBackend();
  const [wcsAvailable, setWcsAvailable] = useState<boolean | null>(null);
  const [wcsInfo, setWcsInfo] = useState<any>(null);
  const [coord, setCoord] = useState<{ ra: number; dec: number } | null>(null);
  const abortRef = useRef(0);
  const lastRequestRef = useRef<string>("");
  const throttleRef = useRef<number | null>(null);
  const busyRef = useRef(false);

  useEffect(() => {
    if (!filePath) {
      setWcsAvailable(null);
      setWcsInfo(null);
      return;
    }
    const seq = ++abortRef.current;
    getWcsInfo(filePath)
      .then((info: any) => {
        if (abortRef.current !== seq) return;
        setWcsAvailable(true);
        setWcsInfo(info);
      })
      .catch(() => {
        if (abortRef.current !== seq) return;
        setWcsAvailable(false);
      });
  }, [filePath, getWcsInfo]);

  useEffect(() => {
    if (!wcsAvailable || !filePath || mouseX === null || mouseY === null) {
      setCoord(null);
      return;
    }

    const key = `${mouseX},${mouseY}`;
    if (key === lastRequestRef.current) return;
    lastRequestRef.current = key;

    if (throttleRef.current) {
      cancelAnimationFrame(throttleRef.current);
    }

    throttleRef.current = requestAnimationFrame(() => {
      throttleRef.current = null;
      if (busyRef.current) return;
      busyRef.current = true;
      const seq = ++abortRef.current;
      const mx = mouseX;
      const my = mouseY;
      pixelToWorld(filePath, mx, my)
        .then((result: any) => {
          if (abortRef.current !== seq) return;
          setCoord({ ra: result.ra, dec: result.dec });
        })
        .catch(() => {})
        .finally(() => {
          busyRef.current = false;
        });
    });
  }, [wcsAvailable, filePath, mouseX, mouseY, pixelToWorld]);

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
