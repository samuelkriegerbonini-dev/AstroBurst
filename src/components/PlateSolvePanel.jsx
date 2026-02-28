import { useState, useCallback, useEffect } from "react";
import { Crosshair, Star, Loader2, Eye, EyeOff } from "lucide-react";

export default function PlateSolvePanel({
  stars = [],
  count = 0,
  isLoading = false,
  onDetect,
  backgroundMedian,
  backgroundSigma,
  imageWidth,
  imageHeight,
  elapsed = 0,
  overlayCanvasRef,
}) {
  const [sigma, setSigma] = useState(5.0);
  const [showOverlay, setShowOverlay] = useState(true);
  const [selectedStar, setSelectedStar] = useState(null);

  useEffect(() => {
    const canvas = overlayCanvasRef?.current;
    if (!canvas || stars.length === 0) return;

    const parent = canvas.parentElement;
    if (!parent) return;

    const W = parent.clientWidth;
    const H = parent.clientHeight;
    canvas.width = W;
    canvas.height = H;

    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, W, H);

    if (!showOverlay) return;

    const scaleX = W / (imageWidth || 1);
    const scaleY = H / (imageHeight || 1);

    const maxFlux = stars.length > 0 ? stars[0].flux : 1;

    stars.forEach((star, i) => {
      const sx = star.x * scaleX;
      const sy = star.y * scaleY;
      const radius = Math.max(3, (star.fwhm || 3) * scaleX * 1.5);
      const brightness = Math.min(1, 0.3 + (star.flux / maxFlux) * 0.7);

      let color;
      if (star.snr > 50) color = `rgba(0, 255, 100, ${brightness})`;
      else if (star.snr > 20) color = `rgba(255, 255, 0, ${brightness})`;
      else color = `rgba(255, 100, 0, ${brightness})`;

      ctx.strokeStyle = color;
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.arc(sx, sy, radius, 0, Math.PI * 2);
      ctx.stroke();

      if (i < 20) {
        ctx.fillStyle = color;
        ctx.font = "9px monospace";
        ctx.fillText(`${i + 1}`, sx + radius + 2, sy - 2);
      }
    });

    if (selectedStar !== null && selectedStar < stars.length) {
      const s = stars[selectedStar];
      const sx = s.x * scaleX;
      const sy = s.y * scaleY;
      const radius = Math.max(6, (s.fwhm || 3) * scaleX * 2);

      ctx.strokeStyle = "rgba(100, 200, 255, 1)";
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.arc(sx, sy, radius, 0, Math.PI * 2);
      ctx.stroke();

      ctx.strokeStyle = "rgba(100, 200, 255, 0.5)";
      ctx.lineWidth = 0.5;
      ctx.beginPath();
      ctx.moveTo(sx - radius * 2, sy);
      ctx.lineTo(sx + radius * 2, sy);
      ctx.moveTo(sx, sy - radius * 2);
      ctx.lineTo(sx, sy + radius * 2);
      ctx.stroke();
    }
  }, [stars, showOverlay, selectedStar, imageWidth, imageHeight, overlayCanvasRef]);

  useEffect(() => {
    const canvas = overlayCanvasRef?.current;
    if (!canvas) return;
    canvas.style.display = showOverlay && stars.length > 0 ? "block" : "none";
  }, [showOverlay, stars.length, overlayCanvasRef]);

  const handleDetect = useCallback(() => {
    if (onDetect) onDetect(sigma);
  }, [onDetect, sigma]);

  const medianFwhm = stars.length > 0
    ? (stars.reduce((s, st) => s + st.fwhm, 0) / stars.length).toFixed(2)
    : "—";

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Crosshair size={12} className="text-cyan-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Star Detection
          </span>
        </div>
        <div className="flex items-center gap-2">
          {stars.length > 0 && (
            <button
              onClick={() => setShowOverlay(!showOverlay)}
              className="text-zinc-500 hover:text-zinc-300 transition-colors"
              title={showOverlay ? "Hide overlay" : "Show overlay"}
            >
              {showOverlay ? <Eye size={12} /> : <EyeOff size={12} />}
            </button>
          )}
        </div>
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="flex items-center gap-2">
          <label className="text-[10px] text-zinc-500 w-12">σ thresh</label>
          <input
            type="range"
            min="2"
            max="15"
            step="0.5"
            value={sigma}
            onChange={(e) => setSigma(parseFloat(e.target.value))}
            className="flex-1 h-1 accent-cyan-500"
          />
          <span className="text-[10px] text-zinc-400 font-mono w-8 text-right">
            {sigma.toFixed(1)}
          </span>
        </div>

        <button
          onClick={handleDetect}
          disabled={isLoading}
          className="w-full flex items-center justify-center gap-2 bg-cyan-600/20 hover:bg-cyan-600/30 text-cyan-300 border border-cyan-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
        >
          {isLoading ? (
            <>
              <Loader2 size={12} className="animate-spin" />
              Detecting...
            </>
          ) : (
            <>
              <Star size={12} />
              Detect Stars
            </>
          )}
        </button>

        {stars.length > 0 && (
          <div className="space-y-1.5">
            <div className="grid grid-cols-3 gap-1 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Stars</div>
                <div className="text-cyan-300 font-mono">{count}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">FWHM</div>
                <div className="text-cyan-300 font-mono">{medianFwhm} px</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Time</div>
                <div className="text-cyan-300 font-mono">{elapsed} ms</div>
              </div>
            </div>

            {backgroundMedian != null && (
              <div className="flex gap-2 text-[10px] text-zinc-500">
                <span>BG: {backgroundMedian.toFixed(1)}</span>
                <span>σ: {backgroundSigma.toFixed(2)}</span>
              </div>
            )}

            <div className="max-h-[120px] overflow-y-auto">
              <table className="w-full text-[10px]">
                <thead>
                  <tr className="text-zinc-500 border-b border-zinc-800/50">
                    <th className="text-left px-1 py-0.5">#</th>
                    <th className="text-right px-1">X</th>
                    <th className="text-right px-1">Y</th>
                    <th className="text-right px-1">Flux</th>
                    <th className="text-right px-1">FWHM</th>
                    <th className="text-right px-1">SNR</th>
                  </tr>
                </thead>
                <tbody>
                  {stars.slice(0, 20).map((s, i) => (
                    <tr
                      key={i}
                      className={`cursor-pointer hover:bg-zinc-800/50 transition-colors ${
                        selectedStar === i ? "bg-cyan-900/20" : ""
                      }`}
                      onClick={() => setSelectedStar(selectedStar === i ? null : i)}
                    >
                      <td className="text-zinc-500 px-1 py-0.5">{i + 1}</td>
                      <td className="text-zinc-300 text-right px-1 font-mono">
                        {s.x.toFixed(1)}
                      </td>
                      <td className="text-zinc-300 text-right px-1 font-mono">
                        {s.y.toFixed(1)}
                      </td>
                      <td className="text-zinc-300 text-right px-1 font-mono">
                        {s.flux.toFixed(0)}
                      </td>
                      <td className="text-zinc-300 text-right px-1 font-mono">
                        {s.fwhm.toFixed(1)}
                      </td>
                      <td className="text-zinc-300 text-right px-1 font-mono">
                        {s.snr.toFixed(0)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
