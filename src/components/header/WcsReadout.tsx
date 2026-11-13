import { useState, useEffect, useCallback, memo } from "react";
import { Globe } from "lucide-react";
import { getWcsInfo, pixelToWorld } from "../../services/astrometry";
import type { WcsInfo, SkyFrame } from "../../shared/types/astrometry";
import { formatLat, formatLon, frameAxisLabels, frameLonInHours, pixelScaleText, type CoordFormat } from "../../utils/coordFormat";
import { ZERO_BASED_PIXEL_TITLE, zeroBasedPixelText } from "../../utils/regionGeometry";
import { ROTATION_TITLE, orientationLine } from "../../utils/wcsOrientation";

interface WcsReadoutProps {
  filePath: string | null;
  imageWidth: number;
  imageHeight: number;
  mouseX: number | null;
  mouseY: number | null;
}

interface ReadoutPreference {
  frame: SkyFrame;
  format: CoordFormat;
}

interface CoordinateResult {
  filePath: string;
  frame: SkyFrame;
  point: [number, number] | null;
}

const READOUT_STORAGE_KEY = "astroburst.readout.v1";
const DEFAULT_READOUT: ReadoutPreference = { frame: "icrs", format: "sexagesimal" };
const FRAMES: readonly SkyFrame[] = ["icrs", "fk5", "galactic", "ecliptic"];
const FORMATS: readonly CoordFormat[] = ["sexagesimal", "decimal"];
const HOVER_DEBOUNCE_MS = 40;
const NO_SKY_POSITION: [number, number] = [Number.NaN, Number.NaN];
const CENTRE_LABEL = "centre";

const SELECT_CLASS =
  "min-h-[22px] max-w-full bg-zinc-900 border border-zinc-700 rounded px-1 text-[10px] text-zinc-300 focus:border-zinc-500";

function loadReadoutPreference(): ReadoutPreference {
  try {
    const text = window.localStorage.getItem(READOUT_STORAGE_KEY);
    if (!text) return DEFAULT_READOUT;
    const raw = JSON.parse(text) as Partial<ReadoutPreference>;
    return {
      frame: FRAMES.find((f) => f === raw.frame) ?? DEFAULT_READOUT.frame,
      format: FORMATS.find((f) => f === raw.format) ?? DEFAULT_READOUT.format,
    };
  } catch {
    return DEFAULT_READOUT;
  }
}

function saveReadoutPreference(pref: ReadoutPreference): void {
  try {
    window.localStorage.setItem(READOUT_STORAGE_KEY, JSON.stringify(pref));
  } catch {
  }
}

function WcsReadoutInner({ filePath, imageWidth, imageHeight, mouseX, mouseY }: WcsReadoutProps) {
  const [wcs, setWcs] = useState<{ filePath: string; info: WcsInfo } | null>(null);
  const [coordResult, setCoord] = useState<CoordinateResult | null>(null);
  const [centerResult, setCenter] = useState<CoordinateResult | null>(null);
  const [pref, setPref] = useState<ReadoutPreference>(loadReadoutPreference);
  const wcsInfo = wcs?.filePath === filePath ? wcs.info : null;
  const coordResolved = coordResult?.filePath === filePath && coordResult.frame === pref.frame;
  const center = centerResult?.filePath === filePath && centerResult.frame === pref.frame ? centerResult.point : null;

  const updatePref = useCallback((patch: Partial<ReadoutPreference>) => {
    setPref((prev) => {
      const next = { ...prev, ...patch };
      saveReadoutPreference(next);
      return next;
    });
  }, []);

  useEffect(() => {
    if (!filePath) {
      setWcs(null);
      setCoord(null);
      return;
    }
    let cancelled = false;
    getWcsInfo(filePath)
      .then((info) => {
        if (cancelled) return;
        setWcs({ filePath, info });
      })
      .catch(() => {
        if (cancelled) return;
        setWcs(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath]);

  useEffect(() => {
    if (!filePath || !wcsInfo) {
      setCenter(null);
      return;
    }
    if (pref.frame === "icrs") {
      setCenter({ filePath, frame: pref.frame, point: [wcsInfo.center_ra, wcsInfo.center_dec] });
      return;
    }
    let cancelled = false;
    pixelToWorld(filePath, [[imageWidth / 2, imageHeight / 2]], pref.frame)
      .then((res) => {
        if (cancelled) return;
        setCenter({ filePath, frame: pref.frame, point: res.points[0] ?? null });
      })
      .catch(() => {
        if (cancelled) return;
        setCenter(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, wcsInfo, pref.frame, imageWidth, imageHeight]);

  useEffect(() => {
    if (!filePath || !wcsInfo || mouseX === null || mouseY === null) {
      setCoord(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      pixelToWorld(filePath, [[mouseX, mouseY]], pref.frame)
        .then((res) => {
          if (cancelled) return;
          setCoord({ filePath, frame: pref.frame, point: res.points[0] ?? null });
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
  }, [filePath, wcsInfo, mouseX, mouseY, pref.frame]);

  if (!wcsInfo) return null;

  const [lonLabel, latLabel] = frameAxisLabels(pref.frame);
  const hours = frameLonInHours(pref.frame);
  const hovering = mouseX !== null && mouseY !== null;
  const hovered = coordResolved ? (coordResult.point ?? NO_SKY_POSITION) : null;
  const shown = hovering ? hovered : center;
  const scale = pixelScaleText(wcsInfo.pixel_scale_arcsec);
  const orientation = orientationLine(wcsInfo);

  return (
    <div className="flex flex-col gap-1 text-[10px] font-mono" style={{ color: "#6ee7b7" }}>
      <div className="flex items-center gap-2 flex-wrap">
        <Globe size={10} />
        <select
          className={SELECT_CLASS}
          value={pref.frame}
          onChange={(e) => updatePref({ frame: e.target.value as SkyFrame })}
          title="Coordinate frame"
          aria-label="Coordinate frame"
        >
          {FRAMES.map((f) => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
        <select
          className={SELECT_CLASS}
          value={pref.format}
          onChange={(e) => updatePref({ format: e.target.value as CoordFormat })}
          title="Coordinate format"
          aria-label="Coordinate format"
        >
          {FORMATS.map((f) => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
        {scale && <span>{scale}</span>}
      </div>
      <div className="flex items-center gap-3 flex-wrap">
        {!hovering && shown && <span className="text-zinc-400">{CENTRE_LABEL}</span>}
        {shown ? (
          <>
            <span>{lonLabel} {formatLon(shown[0], { hours, format: pref.format })}</span>
            <span>{latLabel} {formatLat(shown[1], { format: pref.format })}</span>
          </>
        ) : null}
        {mouseX !== null && mouseY !== null && (
          <span className="text-zinc-400" title={ZERO_BASED_PIXEL_TITLE}>
            {zeroBasedPixelText(mouseX, mouseY)}
          </span>
        )}
      </div>
      {orientation && <div className="text-zinc-400" title={ROTATION_TITLE}>{orientation}</div>}
    </div>
  );
}

export default memo(WcsReadoutInner);
