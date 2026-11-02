import { useEffect, useRef, useState } from "react";
import type { SaguiRegion, SaguiResult } from "../../shared/types/sagui";
import { PREP_FIELD, PrepField } from "../cubeprep/CubePrepStepEditor";
import { regionAt, regionColor, sedDomain } from "./saguiPlot";

function SedPlot({ region, unit }: { region: SaguiRegion; unit: string }) {
  const bands = [...region.bands].filter((band) => Number.isFinite(band.flux) && Number.isFinite(band.pivot_um)).sort((a, b) => a.pivot_um - b.pivot_um);
  const domain = sedDomain(bands);
  const x = (value: number) => 54 + (value - domain.xMin) / (domain.xMax - domain.xMin) * 274;
  const y = (value: number) => 186 - (value - domain.yMin) / (domain.yMax - domain.yMin) * 164;
  const unitLabel = unit;
  return <div className="space-y-2">
    <svg viewBox="0 0 350 226" role="img" aria-label={`Region ${region.region} spectral energy distribution`} className="w-full rounded bg-zinc-950">
      <path d="M54 22V186H328" fill="none" stroke="#71717a" />
      {[0, 0.5, 1].map((fraction) => { const value = domain.yMin + fraction * (domain.yMax - domain.yMin); return <g key={fraction}><line x1={54} x2={328} y1={y(value)} y2={y(value)} stroke="#27272a" /><text x={49} y={y(value) + 3} textAnchor="end" fill="#a1a1aa" fontSize={8}>{value.toExponential(1)}</text></g>; })}
      <polyline points={bands.map((band) => `${x(band.pivot_um)},${y(band.flux)}`).join(" ")} fill="none" stroke="#67e8f9" />
      {bands.map((band) => <g key={band.filter}>
        {band.error !== null && Number.isFinite(band.error) && band.error >= 0 && <path d={`M${x(band.pivot_um)} ${y(band.flux - band.error)}V${y(band.flux + band.error)}M${x(band.pivot_um) - 3} ${y(band.flux - band.error)}h6M${x(band.pivot_um) - 3} ${y(band.flux + band.error)}h6`} stroke="#67e8f9" fill="none" />}
        <circle cx={x(band.pivot_um)} cy={y(band.flux)} r={3} fill="#67e8f9"><title>{`${band.filter}: ${band.flux} ${unitLabel}${band.error === null ? "; uncertainty unknown" : ` ± ${band.error}`}`}</title></circle>
        <text x={x(band.pivot_um)} y={199} textAnchor="middle" fill="#a1a1aa" fontSize={8}>{band.pivot_um.toPrecision(3)}</text>
      </g>)}
      <text x={191} y={218} textAnchor="middle" fill="#d4d4d8" fontSize={10}>Pivot wavelength (µm)</text><text x={56} y={13} fill="#d4d4d8" fontSize={9}>Summed flux ({unitLabel || "input flux units"})</text>
    </svg>
    <div className="overflow-x-auto"><table className="w-full text-[10px]"><thead><tr className="text-zinc-500"><th className="text-left">Filter</th><th className="text-right">µm</th><th className="text-right">Flux</th><th className="text-right">Uncertainty</th></tr></thead><tbody>{bands.map((band) => <tr key={band.filter}><td>{band.filter}</td><td className="text-right">{band.pivot_um.toPrecision(4)}</td><td className="text-right font-mono">{band.flux.toExponential(4)}</td><td className="text-right font-mono">{band.error === null ? "unknown" : band.error.toExponential(4)}</td></tr>)}</tbody></table></div>
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
