import { useState, useEffect, useCallback, memo } from "react";
import { Globe } from "lucide-react";
import { getWcsInfo, pixelToWorld } from "../../services/astrometry";
import type { WcsInfo, SkyFrame } from "../../shared/types/astrometry";
import { formatLat, formatLon, frameAxisLabels, frameLonInHours, type CoordFormat } from "../../utils/coordFormat";
import { ZERO_BASED_PIXEL_TITLE, zeroBasedPixelText } from "../../utils/regionGeometry";

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

const READOUT_STORAGE_KEY = "astroburst.readout.v1";
const DEFAULT_READOUT: ReadoutPreference = { frame: "icrs", format: "sexagesimal" };
const FRAMES: readonly SkyFrame[] = ["icrs", "fk5", "galactic", "ecliptic"];
const FORMATS: readonly CoordFormat[] = ["sexagesimal", "decimal"];
const HOVER_DEBOUNCE_MS = 40;

const SELECT_CLASS =
  "bg-transparent border border-zinc-800 rounded px-0.5 text-[9px] text-zinc-400 focus:border-zinc-600";

const SCALE_DIGITS = 3;
const ROTATION_DIGITS = 1;

function orientationLine(info: WcsInfo): string | null {
  const { projection, pixel_scale_x_arcsec, pixel_scale_y_arcsec, rotation_deg, flipped, sip_present } = info;
  if (
    !projection ||
    pixel_scale_x_arcsec === undefined ||
    pixel_scale_y_arcsec === undefined ||
    rotation_deg === undefined ||
    flipped === undefined
  ) {
    return null;
  }
  const scale = `${pixel_scale_x_arcsec.toFixed(SCALE_DIGITS)}"/px x ${pixel_scale_y_arcsec.toFixed(SCALE_DIGITS)}"/px`;
  const east = flipped ? "right" : "left";
  return `${projection} - ${scale} - rot ${rotation_deg.toFixed(ROTATION_DIGITS)} deg - E ${east}${sip_present ? " - SIP" : ""}`;
}

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
  const [wcsAvailable, setWcsAvailable] = useState<boolean | null>(null);
  const [wcsInfo, setWcsInfo] = useState<WcsInfo | null>(null);
  const [coord, setCoord] = useState<[number, number] | null>(null);
  const [center, setCenter] = useState<[number, number] | null>(null);
  const [pref, setPref] = useState<ReadoutPreference>(loadReadoutPreference);

  const updatePref = useCallback((patch: Partial<ReadoutPreference>) => {
    setPref((prev) => {
      const next = { ...prev, ...patch };
      saveReadoutPreference(next);
      return next;
    });
  }, []);

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
    if (!filePath || !wcsAvailable || !wcsInfo) {
      setCenter(null);
      return;
    }
    if (pref.frame === "icrs") {
      setCenter([wcsInfo.center_ra, wcsInfo.center_dec]);
      return;
    }
    let cancelled = false;
    pixelToWorld(filePath, [[imageWidth / 2, imageHeight / 2]], pref.frame)
      .then((res) => {
        if (cancelled) return;
        setCenter(res.points[0] ?? null);
      })
      .catch(() => {
        if (cancelled) return;
        setCenter(null);
      });
    return () => {
      cancelled = true;
    };
  }, [filePath, wcsAvailable, wcsInfo, pref.frame, imageWidth, imageHeight]);

  useEffect(() => {
    if (!filePath || !wcsAvailable || mouseX === null || mouseY === null) {
      setCoord(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      pixelToWorld(filePath, [[mouseX, mouseY]], pref.frame)
        .then((res) => {
          if (cancelled) return;
          setCoord(res.points[0] ?? null);
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
  }, [filePath, wcsAvailable, mouseX, mouseY, pref.frame]);

  if (!wcsAvailable || !wcsInfo) return null;

  const [lonLabel, latLabel] = frameAxisLabels(pref.frame);
  const hours = frameLonInHours(pref.frame);
  const shown = coord ?? center;
  const orientation = orientationLine(wcsInfo);

  return (
    <div className="flex flex-col gap-1 text-[10px] font-mono" style={{ color: "rgba(52,211,153,0.6)" }}>
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
        {wcsInfo.pixel_scale_arcsec && (
          <span>{wcsInfo.pixel_scale_arcsec.toFixed(2)}"/px</span>
        )}
      </div>
      <div className="flex items-center gap-3 flex-wrap">
        {shown ? (
          <>
            <span>{lonLabel} {formatLon(shown[0], { hours, format: pref.format })}</span>
            <span>{latLabel} {formatLat(shown[1], { format: pref.format })}</span>
          </>
        ) : null}
        {mouseX !== null && mouseY !== null && (
          <span className="text-zinc-600" title={ZERO_BASED_PIXEL_TITLE}>
            {zeroBasedPixelText(mouseX, mouseY)}
          </span>
        )}
      </div>
      {orientation && <div className="text-zinc-600">{orientation}</div>}
    </div>
  );
}

export default memo(WcsReadoutInner);
