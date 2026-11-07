import { useEffect, useRef, useState } from "react";
import type { SaguiRegion, SaguiRegionalBand, SaguiResult } from "../../shared/types/sagui";
import { PREP_FIELD, PrepField } from "../cubeprep/CubePrepStepEditor";
import { regionAt, regionColor, sedDomain } from "./saguiPlot";

const PLOT = { left: 54, right: 328, top: 22, bottom: 186 };
const PIVOT_LABEL_GAP = 20;
const LOG_PAD = 0.1;
const MIN_LOG_DECADES = 1;
const MAX_DECADE_TICKS = 6;
const LOG_MANTISSAS = [1, 2, 5];
const LINEAR_TICKS = [0, 0.5, 1];
const PAINT = {
  axis: { stroke: "var(--ab-text-4)" },
  grid: { stroke: "var(--ab-border-strong)" },
  label: { fill: "var(--ab-text-2)" },
  title: { fill: "var(--ab-text-1)" },
  line: { stroke: "var(--ab-cyan)" },
  point: { fill: "var(--ab-cyan)" },
};

interface FluxAxis {
  log: boolean;
  y: (flux: number) => number;
  ticks: { value: number; label: string }[];
}

const spread = (band: SaguiRegionalBand) => band.error !== null && Number.isFinite(band.error) && band.error >= 0 ? band.error : 0;

function logTicks(low: number, high: number): number[] {
  const decades: number[] = [];
  for (let exponent = Math.ceil(low); exponent <= Math.floor(high); exponent++) decades.push(exponent);
  if (decades.length >= 2) {
    const stride = Math.ceil(decades.length / MAX_DECADE_TICKS);
    return decades.filter((_, index) => index % stride === 0).map((exponent) => 10 ** exponent);
  }
  const ticks: number[] = [];
  for (let exponent = Math.floor(low); exponent <= Math.ceil(high); exponent++) {
    for (const mantissa of LOG_MANTISSAS) {
      const position = Math.log10(mantissa * 10 ** exponent);
      if (position >= low && position <= high) ticks.push(mantissa * 10 ** exponent);
    }
  }
  return ticks;
}

function fluxAxis(bands: SaguiRegionalBand[]): FluxAxis {
  const height = PLOT.bottom - PLOT.top;
  if (bands.length > 0 && bands.every((band) => band.flux > 0)) {
    const logs = bands.flatMap((band) => [Math.log10(band.flux - spread(band) > 0 ? band.flux - spread(band) : band.flux), Math.log10(band.flux + spread(band))]);
    const span = Math.max(...logs) - Math.min(...logs);
    const pad = Math.max(span * LOG_PAD, (MIN_LOG_DECADES - span) / 2);
    const low = Math.min(...logs) - pad;
    const high = Math.max(...logs) + pad;
    const y = (flux: number) => flux > 0 ? PLOT.bottom - (Math.log10(flux) - low) / (high - low) * height : PLOT.bottom;
    return { log: true, y, ticks: logTicks(low, high).map((value) => ({ value, label: value.toExponential(0) })) };
  }
  const domain = sedDomain(bands);
  const y = (flux: number) => PLOT.bottom - (flux - domain.yMin) / (domain.yMax - domain.yMin) * height;
  return { log: false, y, ticks: LINEAR_TICKS.map((fraction) => domain.yMin + fraction * (domain.yMax - domain.yMin)).map((value) => ({ value, label: value.toExponential(1) })) };
}

function spacedLabels(positions: number[]): Set<number> {
  const kept = new Set<number>();
  let last = -Infinity;
  positions.forEach((position, index) => {
    if (position - last < PIVOT_LABEL_GAP) return;
    kept.add(index);
    last = position;
  });
  return kept;
}

function SedPlot({ region, unit }: { region: SaguiRegion; unit: string }) {
  const bands = [...region.bands].filter((band) => Number.isFinite(band.flux) && Number.isFinite(band.pivot_um)).sort((a, b) => a.pivot_um - b.pivot_um);
  const domain = sedDomain(bands);
  const x = (value: number) => PLOT.left + (value - domain.xMin) / (domain.xMax - domain.xMin) * (PLOT.right - PLOT.left);
  const axis = fluxAxis(bands);
  const y = axis.y;
  const labelled = spacedLabels(bands.map((band) => x(band.pivot_um)));
  const unitLabel = unit;
  return <div className="space-y-2">
    <svg viewBox="0 0 350 226" role="img" aria-label={`Region ${region.region} spectral energy distribution`} className="w-full rounded bg-zinc-950">
      <path d={`M${PLOT.left} ${PLOT.top}V${PLOT.bottom}H${PLOT.right}`} fill="none" style={PAINT.axis} />
      {axis.ticks.map((tick) => <g key={tick.value}><line x1={PLOT.left} x2={PLOT.right} y1={y(tick.value)} y2={y(tick.value)} style={PAINT.grid} /><text x={PLOT.left - 5} y={y(tick.value) + 3} textAnchor="end" style={PAINT.label} fontSize={8}>{tick.label}</text></g>)}
      <polyline points={bands.map((band) => `${x(band.pivot_um)},${y(band.flux)}`).join(" ")} fill="none" style={PAINT.line} />
      {bands.map((band, index) => <g key={band.filter}>
        {band.error !== null && Number.isFinite(band.error) && band.error >= 0 && <path d={`M${x(band.pivot_um)} ${y(band.flux - band.error)}V${y(band.flux + band.error)}M${x(band.pivot_um) - 3} ${y(band.flux - band.error)}h6M${x(band.pivot_um) - 3} ${y(band.flux + band.error)}h6`} fill="none" style={PAINT.line} />}
        <circle cx={x(band.pivot_um)} cy={y(band.flux)} r={3} style={PAINT.point}><title>{`${band.filter}: ${band.flux} ${unitLabel}${band.error === null ? "; uncertainty unknown" : ` ± ${band.error}`}`}</title></circle>
        {labelled.has(index) && <text x={x(band.pivot_um)} y={199} textAnchor="middle" style={PAINT.label} fontSize={8}>{band.pivot_um.toPrecision(3)}</text>}
      </g>)}
      <text x={191} y={218} textAnchor="middle" style={PAINT.title} fontSize={10}>Pivot wavelength (µm)</text><text x={56} y={13} style={PAINT.title} fontSize={9}>Summed flux ({unitLabel || "input flux units"}){axis.log ? ", log scale" : ""}</text>
    </svg>
    <div className="overflow-x-auto"><table className="w-full text-[10px]"><thead><tr className="text-zinc-500"><th className="text-left">Filter</th><th className="text-right">µm</th><th className="text-right">Flux</th><th className="text-right">Uncertainty</th><th className="text-right">Correlated</th></tr></thead><tbody>{bands.map((band) => <tr key={band.filter}><td>{band.filter}</td><td className="text-right">{band.pivot_um.toPrecision(4)}</td><td className="text-right font-mono">{band.flux.toExponential(4)}</td><td className="text-right font-mono">{band.error === null ? "unknown" : band.error.toExponential(4)}</td><td className="text-right font-mono">{typeof band.error_correlated === "number" ? band.error_correlated.toExponential(4) : "unavailable"}</td></tr>)}</tbody></table></div>
    <p className="text-[10px] text-zinc-500">Correlated uncertainty includes the noise correlation from resampling and PSF matching; Uncertainty assumes independent pixels.</p>
  </div>;
}

export default function SaguiViewer({ result }: { result: SaguiResult }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [selection, setSelection] = useState<number | null>(null);
  const selected = result.regions.find((region) => region.region === selection) ?? result.regions[0];
  useEffect(() => {
    const element = canvas.current;
    const context = element?.getContext("2d");
    if (!element || !context || result.width <= 0 || result.height <= 0) return;
    const scale = Math.min(1, 1024 / Math.max(result.width, result.height));
    element.width = Math.max(1, Math.round(result.width * scale));
    element.height = Math.max(1, Math.round(result.height * scale));
    const pixels = context.createImageData(element.width, element.height);
    const colors = new Map<number, [number, number, number]>();
    for (let row = 0; row < element.height; row++) for (let col = 0; col < element.width; col++) {
      const region = regionAt(result, (col + 0.5) / element.width, (row + 0.5) / element.height) ?? 0;
      if (!colors.has(region)) colors.set(region, regionColor(region));
      const color = colors.get(region)!;
      const offset = (row * element.width + col) * 4;
      const dim = selected && selected.region !== region && region !== 0 ? 0.5 : 1;
      pixels.data[offset] = color[0] * dim;
      pixels.data[offset + 1] = color[1] * dim;
      pixels.data[offset + 2] = color[2] * dim;
      pixels.data[offset + 3] = 255;
    }
    context.putImageData(pixels, 0, 0);
  }, [result, selected]);
  return <div className="space-y-3">
    <h3 className="font-semibold text-cyan-200">Regions and SEDs</h3>
    <canvas ref={canvas} role="img" aria-label="Categorical region map; select a region with the menu below" className="w-full max-h-96 object-contain rounded bg-zinc-950 cursor-crosshair" style={{ imageRendering: "pixelated" }} onClick={(event) => {
      const bounds = event.currentTarget.getBoundingClientRect();
      const canvasRatio = result.width / result.height;
      const displayWidth = Math.min(bounds.width, bounds.height * canvasRatio);
      const displayHeight = displayWidth / canvasRatio;
      const x = (event.clientX - bounds.left - (bounds.width - displayWidth) / 2) / displayWidth;
      const y = (event.clientY - bounds.top - (bounds.height - displayHeight) / 2) / displayHeight;
      const region = regionAt(result, x, y);
      if (region !== null && region > 0) setSelection(region);
    }} />
    <p className="text-[10px] text-zinc-500">Categorical colors; dark background = excluded pixels. X increases right, Y upward. Click a region or select it below.</p>
    <PrepField label="Selected region"><select className={PREP_FIELD} value={selected?.region ?? ""} onChange={(e) => setSelection(Number(e.target.value))}>{result.regions.map((region) => <option key={region.region} value={region.region}>Region {region.region} · {region.n_pixels} pixels</option>)}</select></PrepField>
    {selected && <SedPlot region={selected} unit={result.unit} />}
  </div>;
}
