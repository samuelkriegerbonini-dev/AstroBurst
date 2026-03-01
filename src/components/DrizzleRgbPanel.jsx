/**
 * @fileoverview DrizzleRgbPanel - Integrated RGB Drizzle Pipeline Component
 * 
 * This component provides a unified interface for performing drizzle stacking
 * on RGB astronomical images. It combines three separate drizzle operations
 * (one per channel) with automatic RGB composition into a single workflow.
 * 
 * @description
 * The drizzle algorithm, originally developed for HST (Hubble Space Telescope),
 * allows sub-pixel reconstruction from dithered observations. This panel extends
 * the standard grayscale drizzle to handle tricolor imaging workflows commonly
 * used in narrowband astrophotography (e.g., Hubble Palette: Hα→Red, OIII→Green, SII→Blue).
 * 
 * @algorithm Drizzle (Variable Pixel Linear Reconstruction)
 * The drizzle algorithm maps input pixels onto an output grid with sub-pixel precision:
 * 
 *   output[x',y'] = Σ w_i * input_i[x,y] / Σ w_i
 * 
 * Where:
 *   - w_i is the overlap area between the shrunken input pixel and output pixel
 *   - pixfrac controls the linear shrink factor of input pixels
 *   - scale determines the output grid resolution multiplier
 * 
 * @reference
 * Fruchter, A.S. & Hook, R.N. (2002). "Drizzle: A Method for the Linear 
 * Reconstruction of Undersampled Images". PASP 114, 144-152.
 * DOI: 10.1086/338393
 * 
 * @see https://www.stsci.edu/scientific-community/software/drizzlepac
 * 
 * @module components/DrizzleRgbPanel
 * @version 0.1.0
 * @author AstroBurst Team
 * @license MIT
 */

import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Layers, Loader2, Maximize2, Info, Palette, ChevronDown, ChevronRight, RefreshCw } from "lucide-react";
import ProgressBar from "./ProgressBar";

/**
 * @typedef {Object} DrizzleRgbOptions
 * @property {number} scale - Output scale factor (1.5, 2.0, or 3.0)
 * @property {number} pixfrac - Pixel fraction / drop shrink factor (0.1 to 1.0)
 * @property {string} kernel - Interpolation kernel: 'square' | 'gaussian' | 'lanczos3'
 * @property {number} sigmaLow - Lower sigma clipping threshold for outlier rejection
 * @property {number} sigmaHigh - Upper sigma clipping threshold for outlier rejection
 * @property {boolean} align - Enable sub-pixel alignment via ZNCC cross-correlation
 * @property {string} wbMode - White balance mode: 'auto' | 'none' | 'manual'
 * @property {boolean} scnrEnabled - Enable SCNR (Subtractive Chromatic Noise Reduction)
 * @property {string} scnrMethod - SCNR method: 'average' | 'maximum'
 * @property {number} scnrAmount - SCNR strength (0.0 to 1.0)
 */

/**
 * @typedef {Object} DrizzleRgbResult
 * @property {string} png_path - Path to the output PNG preview
 * @property {string} previewUrl - Converted preview URL for display
 * @property {string} fits_path - Path to the output FITS file
 * @property {[number, number]} input_dims - Original input dimensions [width, height]
 * @property {[number, number]} output_dims - Drizzled output dimensions [width, height]
 * @property {number} frame_count_r - Number of frames processed for red channel
 * @property {number} frame_count_g - Number of frames processed for green channel
 * @property {number} frame_count_b - Number of frames processed for blue channel
 * @property {number} rejected_pixels - Total rejected pixels across all channels
 * @property {number} elapsed_ms - Total processing time in milliseconds
 * @property {Object} stats_r - Statistics for red channel
 * @property {Object} stats_g - Statistics for green channel
 * @property {Object} stats_b - Statistics for blue channel
 */

/**
 * @typedef {Object} FileEntry
 * @property {string} path - Absolute file path
 * @property {string} name - Display name
 * @property {string} [id] - Unique identifier
 * @property {[number, number]} [dimensions] - Image dimensions if available
 */

/**
 * DrizzleRgbPanel - Main component for RGB drizzle workflow
 * 
 * @description
 * This component orchestrates the complete RGB drizzle pipeline:
 * 1. Channel Assignment: User assigns files to R, G, B channels
 * 2. Drizzle Configuration: Scale, pixfrac, kernel, sigma clipping
 * 3. Processing: Backend processes each channel with drizzle algorithm
 * 4. Composition: Channels are combined with optional white balance and SCNR
 * 
 * @component
 * @param {Object} props - Component properties
 * @param {FileEntry[]} props.files - Array of available FITS files
 * @param {Function} props.onDrizzleRgb - Callback to execute drizzle RGB operation
 * @param {DrizzleRgbResult|null} props.result - Result from last drizzle operation
 * @param {boolean} props.isLoading - Loading state indicator
 * @param {number} props.progress - Progress percentage (0-100)
 * @param {string} props.progressStage - Current processing stage description
 * 
 * @returns {JSX.Element} The rendered DrizzleRgbPanel component
 * 
 * @example
 * <DrizzleRgbPanel
 *   files={processedFiles}
 *   onDrizzleRgb={handleDrizzleRgb}
 *   result={drizzleResult}
 *   isLoading={isProcessing}
 *   progress={45}
 *   progressStage="Processing Red channel..."
 * />
 */
export default function DrizzleRgbPanel({
  files = [],
  onDrizzleRgb,
  result = null,
  isLoading = false,
  progress = 0,
  progressStage = "",
}) {
  /**
   * @state {string[]} rPaths - Selected file paths for red channel
   * @description Array of FITS file paths assigned to the red channel.
   * Multiple files enable drizzle stacking for improved SNR and resolution.
   */
  const [rPaths, setRPaths] = useState([]);

  /**
   * @state {string[]} gPaths - Selected file paths for green channel
   */
  const [gPaths, setGPaths] = useState([]);

  /**
   * @state {string[]} bPaths - Selected file paths for blue channel
   */
  const [bPaths, setBPaths] = useState([]);

  /**
   * @state {number} scale - Drizzle output scale factor
   * @description Controls the output resolution multiplier.
   * - 1.5×: Subtle enhancement, minimal artifacts
   * - 2.0×: Standard drizzle (recommended for dithered data)
   * - 3.0×: Aggressive upscaling, requires well-dithered input
   * 
   * @mathematical_basis
   * Output dimensions: [W_out, H_out] = [W_in × scale, H_in × scale]
   * Each output pixel covers (1/scale)² of the original pixel area.
   */
  const [scale, setScale] = useState(2.0);

  /**
   * @state {number} pixfrac - Pixel fraction (drop size)
   * @description Controls the linear shrink factor applied to input pixels
   * before mapping to the output grid.
   * 
   * @range 0.1 to 1.0
   * @default 0.7
   * 
   * @mathematical_basis
   * The input pixel is shrunk by factor pixfrac before "dropping" onto output:
   *   drop_size = pixel_size × pixfrac
   * 
   * Lower values (0.4-0.6): Sharper results but requires good dithering
   * Higher values (0.8-1.0): Smoother results, more forgiving of alignment
   * 
   * The effective PSF of drizzled output approximates:
   *   σ_out ≈ √(σ_in² + (pixfrac × pixel_scale)²)
   */
  const [pixfrac, setPixfrac] = useState(0.7);

  /**
   * @state {string} kernel - Drizzle interpolation kernel
   * @description Determines how input pixel flux is distributed to output pixels.
   * 
   * Available kernels:
   * - 'square': Original HST drizzle kernel. Weight = overlap area.
   *   Best for point sources and high-frequency detail.
   * 
   * - 'gaussian': Gaussian-weighted distribution.
   *   σ = pixfrac × 0.5, provides smoother transitions.
   *   Better for extended objects and noise suppression.
   * 
   * - 'lanczos3': Lanczos-3 sinc-based kernel.
   *   Provides sharp edges with minimal ringing.
   *   Best for preserving fine structure in nebulae.
   * 
   * @mathematical_basis
   * Square kernel: w(x,y) = overlap_area(input_pixel, output_pixel)
   * Gaussian kernel: w(x,y) = exp(-r²/(2σ²))
   * Lanczos-3: w(x) = sinc(x) × sinc(x/3) for |x| < 3
   */
  const [kernel, setKernel] = useState("square");

  /**
   * @state {number} sigmaLow - Lower sigma clipping threshold
   * @description Pixels below (median - sigmaLow × σ) are rejected as outliers.
   * Used to remove cosmic rays, hot pixels, and artifacts.
   * 
   * @range 1.0 to 10.0
   * @default 3.0
   * 
   * @mathematical_basis
   * For each output pixel stack, iteratively:
   * 1. Compute median and MAD (Median Absolute Deviation)
   * 2. σ_robust = 1.4826 × MAD
   * 3. Reject if value < median - sigmaLow × σ_robust
   */
  const [sigmaLow, setSigmaLow] = useState(3.0);

  /**
   * @state {number} sigmaHigh - Upper sigma clipping threshold
   * @description Pixels above (median + sigmaHigh × σ) are rejected.
   * Primarily removes cosmic ray hits and satellite trails.
   * 
   * @range 1.0 to 10.0
   * @default 3.0
   */
  const [sigmaHigh, setSigmaHigh] = useState(3.0);

  /**
   * @state {boolean} align - Enable sub-pixel alignment
   * @description When enabled, frames are aligned using Zero-mean Normalized
   * Cross-Correlation (ZNCC) before drizzling.
   * 
   * @mathematical_basis
   * ZNCC correlation coefficient:
   *   r = Σ[(A - μ_A)(B - μ_B)] / √[Σ(A - μ_A)² × Σ(B - μ_B)²]
   * 
   * Sub-pixel offset is found by fitting a 2D parabola to the correlation peak.
   */
  const [align, setAlign] = useState(true);

  /**
   * @state {string} wbMode - White balance mode
   * @description Controls how channel intensities are balanced after composition.
   * - 'auto': Equalizes median values across channels
   * - 'none': No white balance applied
   * - 'manual': User-specified RGB multipliers
   */
  const [wbMode, setWbMode] = useState("auto");

  /**
   * @state {boolean} scnrEnabled - Enable SCNR
   * @description Subtractive Chromatic Noise Reduction removes green cast
   * commonly seen in narrowband compositions (Hubble Palette).
   */
  const [scnrEnabled, setScnrEnabled] = useState(false);

  /**
   * @state {string} scnrMethod - SCNR algorithm method
   * @description
   * - 'average': G_new = G - amount × (G - (R + B) / 2)
   * - 'maximum': G_new = G - amount × (G - max(R, B))
   */
  const [scnrMethod, setScnrMethod] = useState("average");

  /**
   * @state {number} scnrAmount - SCNR strength
   * @range 0.0 to 1.0
   * @default 0.5
   */
  const [scnrAmount, setScnrAmount] = useState(0.5);

  /**
   * @state {Object} expandedChannels - UI state for channel accordions
   */
  const [expandedChannels, setExpandedChannels] = useState({ r: true, g: false, b: false });

  /**
   * @state {number} elapsed - Elapsed processing time in seconds
   */
  const [elapsed, setElapsed] = useState(0);

  /**
   * @ref {number|null} timerRef - Reference to the elapsed time interval
   */
  const timerRef = useRef(null);

  /**
   * @effect Timer management for elapsed time display
   * @description Starts a 100ms interval timer when processing begins,
   * clears it when processing completes.
   */
  useEffect(() => {
    if (isLoading) {
      setElapsed(0);
      const start = Date.now();
      timerRef.current = setInterval(() => {
        setElapsed(((Date.now() - start) / 1000).toFixed(1));
      }, 100);
    } else {
      if (timerRef.current) clearInterval(timerRef.current);
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isLoading]);

  /**
   * Toggle file selection for a specific channel
   * 
   * @function toggleFile
   * @param {string} channel - Channel identifier ('r', 'g', or 'b')
   * @param {string} path - File path to toggle
   * 
   * @description Adds or removes a file path from the specified channel's
   * selection array. Uses functional state update to ensure consistency.
   */
  const toggleFile = useCallback((channel, path) => {
    const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
    setter((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path]
    );
  }, []);

  /**
   * Select all files for a specific channel
   * 
   * @function selectAllForChannel
   * @param {string} channel - Channel identifier ('r', 'g', or 'b')
   */
  const selectAllForChannel = useCallback(
    (channel) => {
      const allPaths = files.map((f) => f.path);
      const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
      setter(allPaths);
    },
    [files]
  );

  /**
   * Clear all selections for a specific channel
   * 
   * @function clearChannel
   * @param {string} channel - Channel identifier ('r', 'g', or 'b')
   */
  const clearChannel = useCallback((channel) => {
    const setter = channel === "r" ? setRPaths : channel === "g" ? setGPaths : setBPaths;
    setter([]);
  }, []);

  /**
   * Toggle accordion expansion state for a channel
   * 
   * @function toggleExpand
   * @param {string} channel - Channel identifier ('r', 'g', or 'b')
   */
  const toggleExpand = useCallback((channel) => {
    setExpandedChannels((prev) => ({ ...prev, [channel]: !prev[channel] }));
  }, []);

  /**
   * Auto-assign files to channels based on filter detection
   * 
   * @function handleAutoAssign
   * @description Attempts to automatically assign files to RGB channels
   * based on common filter naming conventions:
   * - Red: Hα, H-alpha, R filter, red, f656n, f658n
   * - Green: OIII, O3, G filter, green, f502n, f501n
   * - Blue: SII, S2, B filter, blue, f673n
   * 
   * Falls back to sequential assignment if no patterns match.
   */
  const handleAutoAssign = useCallback(() => {
    const patterns = {
      r: [/[_-]r[._-]/i, /ha|h.?alpha|red|f656|f658/i, /[_-]R\./],
      g: [/[_-]g[._-]/i, /oiii|o3|green|f502|f501/i, /[_-]G\./],
      b: [/[_-]b[._-]/i, /sii|s2|blue|f673/i, /[_-]B\./],
    };

    const rMatches = [];
    const gMatches = [];
    const bMatches = [];

    for (const f of files) {
      const name = f.name || f.path || "";
      if (patterns.r.some((p) => p.test(name))) rMatches.push(f.path);
      else if (patterns.g.some((p) => p.test(name))) gMatches.push(f.path);
      else if (patterns.b.some((p) => p.test(name))) bMatches.push(f.path);
    }

    if (rMatches.length > 0) setRPaths(rMatches);
    if (gMatches.length > 0) setGPaths(gMatches);
    if (bMatches.length > 0) setBPaths(bMatches);

    if (rMatches.length === 0 && gMatches.length === 0 && bMatches.length === 0) {
      const third = Math.ceil(files.length / 3);
      setRPaths(files.slice(0, third).map((f) => f.path));
      setGPaths(files.slice(third, third * 2).map((f) => f.path));
      setBPaths(files.slice(third * 2).map((f) => f.path));
    }
  }, [files]);

  /**
   * @computed canDrizzle - Validation check for drizzle operation
   * @description Returns true if at least 2 channels have 2+ frames each.
   * Drizzle requires multiple dithered frames for sub-pixel reconstruction.
   */
  const canDrizzle = useMemo(() => {
    const channelsWithFrames = [rPaths.length >= 2, gPaths.length >= 2, bPaths.length >= 2].filter(Boolean).length;
    return channelsWithFrames >= 2;
  }, [rPaths, gPaths, bPaths]);

  /**
   * @computed totalFrames - Total number of frames across all channels
   */
  const totalFrames = useMemo(() => rPaths.length + gPaths.length + bPaths.length, [rPaths, gPaths, bPaths]);

  /**
   * @computed estimatedOutputRes - Estimated output resolution string
   * @description Calculates expected output dimensions based on first file
   * dimensions and selected scale factor.
   */
  const estimatedOutputRes = useMemo(() => {
    if (result) {
      return `${result.output_dims[0]}×${result.output_dims[1]}`;
    }
    const firstFile = files[0];
    if (firstFile?.dimensions) {
      const w = Math.ceil(firstFile.dimensions[0] * scale);
      const h = Math.ceil(firstFile.dimensions[1] * scale);
      return `~${w}×${h}`;
    }
    return null;
  }, [result, files, scale]);

  /**
   * Execute the drizzle RGB operation
   * 
   * @function handleDrizzle
   * @description Validates inputs and calls the backend drizzle RGB command
   * with all configured parameters.
   */
  const handleDrizzle = useCallback(() => {
    if (!canDrizzle || !onDrizzleRgb) return;
    onDrizzleRgb(
      rPaths.length >= 2 ? rPaths : null,
      gPaths.length >= 2 ? gPaths : null,
      bPaths.length >= 2 ? bPaths : null,
      {
        scale,
        pixfrac,
        kernel,
        sigmaLow,
        sigmaHigh,
        align,
        wbMode,
        scnrEnabled,
        scnrMethod,
        scnrAmount,
      }
    );
  }, [
    canDrizzle,
    onDrizzleRgb,
    rPaths,
    gPaths,
    bPaths,
    scale,
    pixfrac,
    kernel,
    sigmaLow,
    sigmaHigh,
    align,
    wbMode,
    scnrEnabled,
    scnrMethod,
    scnrAmount,
  ]);

  /**
   * ChannelAccordion - Collapsible channel assignment UI
   * 
   * @component
   * @param {Object} props
   * @param {string} props.label - Channel label ('R', 'G', 'B')
   * @param {string} props.color - CSS color for the channel
   * @param {string} props.channel - Channel identifier
   * @param {string[]} props.paths - Selected file paths for this channel
   * @param {boolean} props.expanded - Accordion expansion state
   * 
   * @returns {JSX.Element} Rendered accordion section
   */
  const ChannelAccordion = ({ label, color, channel, paths, expanded }) => (
    <div className="border border-zinc-800/50 rounded overflow-hidden">
      <button
        onClick={() => toggleExpand(channel)}
        className="w-full flex items-center justify-between px-2 py-1.5 bg-zinc-900/50 hover:bg-zinc-900 transition-colors"
      >
        <div className="flex items-center gap-2">
          <div
            className="w-3 h-3 rounded-full border-2"
            style={{ backgroundColor: color + "33", borderColor: color }}
          />
          <span className="text-[11px] font-medium text-zinc-300">{label}</span>
          {paths.length > 0 && (
            <span className="text-[10px] text-zinc-500 bg-zinc-800 px-1.5 py-0.5 rounded">
              {paths.length} frames
            </span>
          )}
        </div>
        {expanded ? (
          <ChevronDown size={12} className="text-zinc-500" />
        ) : (
          <ChevronRight size={12} className="text-zinc-500" />
        )}
      </button>

      {expanded && (
        <div className="px-2 py-1.5 bg-zinc-950/50 space-y-1">
          <div className="flex items-center justify-between">
            <span className="text-[9px] text-zinc-600">Select frames for {label} channel</span>
            <div className="flex gap-2">
              <button
                onClick={() => selectAllForChannel(channel)}
                className="text-[9px] text-zinc-500 hover:text-zinc-300"
              >
                All
              </button>
              <button
                onClick={() => clearChannel(channel)}
                className="text-[9px] text-zinc-500 hover:text-zinc-300"
              >
                Clear
              </button>
            </div>
          </div>
          <div className="max-h-24 overflow-y-auto space-y-0.5 custom-scrollbar">
            {files.map((f) => (
              <label
                key={f.path || f.id}
                className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer hover:text-zinc-300 py-0.5"
              >
                <input
                  type="checkbox"
                  checked={paths.includes(f.path)}
                  onChange={() => toggleFile(channel, f.path)}
                  className="w-3 h-3"
                  style={{ accentColor: color }}
                />
                <span className="truncate">{f.name || f.path}</span>
              </label>
            ))}
            {files.length === 0 && (
              <div className="text-[10px] text-zinc-600 py-2 text-center">No FITS files loaded</div>
            )}
          </div>
        </div>
      )}
    </div>
  );

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <div className="relative">
            <Layers size={12} className="text-indigo-400" />
            <Palette size={8} className="text-pink-400 absolute -bottom-0.5 -right-0.5" />
          </div>
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Drizzle RGB
          </span>
        </div>
        <div className="flex items-center gap-2">
          {totalFrames > 0 && (
            <span className="text-[10px] text-indigo-300 bg-indigo-500/20 px-1.5 py-0.5 rounded">
              {totalFrames} total
            </span>
          )}
          {files.length >= 2 && (
            <button
              onClick={handleAutoAssign}
              className="text-[10px] text-zinc-500 hover:text-zinc-300 flex items-center gap-1"
              title="Auto-assign channels by filter detection"
            >
              <RefreshCw size={10} />
              Auto
            </button>
          )}
        </div>
      </div>

      <div className="px-3 py-2 space-y-2">
        {/* Channel Assignments */}
        <div className="space-y-1">
          <ChannelAccordion
            label="Red Channel"
            color="#ef4444"
            channel="r"
            paths={rPaths}
            expanded={expandedChannels.r}
          />
          <ChannelAccordion
            label="Green Channel"
            color="#22c55e"
            channel="g"
            paths={gPaths}
            expanded={expandedChannels.g}
          />
          <ChannelAccordion
            label="Blue Channel"
            color="#3b82f6"
            channel="b"
            paths={bPaths}
            expanded={expandedChannels.b}
          />
        </div>

        {/* Drizzle Parameters */}
        <div className="border-t border-zinc-800/50 pt-2 space-y-1.5">
          <div className="text-[10px] text-zinc-500 uppercase tracking-wider mb-1">
            Drizzle Parameters
          </div>

          {/* Scale */}
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Scale</label>
            <select
              value={scale}
              onChange={(e) => setScale(parseFloat(e.target.value))}
              className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
            >
              <option value="1.5">1.5× (Subtle)</option>
              <option value="2.0">2.0× (Standard)</option>
              <option value="3.0">3.0× (Aggressive)</option>
            </select>
          </div>

          {/* Pixfrac */}
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Pixfrac</label>
            <input
              type="range"
              min="0.1"
              max="1.0"
              step="0.05"
              value={pixfrac}
              onChange={(e) => setPixfrac(parseFloat(e.target.value))}
              className="flex-1 h-1 accent-indigo-500"
            />
            <span className="text-[10px] text-zinc-300 font-mono w-8 text-right">
              {pixfrac.toFixed(2)}
            </span>
          </div>

          {/* Kernel */}
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Kernel</label>
            <select
              value={kernel}
              onChange={(e) => setKernel(e.target.value)}
              className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
            >
              <option value="square">Square (Variable Pixel)</option>
              <option value="gaussian">Gaussian</option>
              <option value="lanczos3">Lanczos-3</option>
            </select>
          </div>

          {/* Sigma Clipping */}
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Sigma</label>
            <div className="flex-1 flex items-center gap-1">
              <input
                type="number"
                min="1"
                max="10"
                step="0.5"
                value={sigmaLow}
                onChange={(e) => setSigmaLow(parseFloat(e.target.value))}
                className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center"
              />
              <span className="text-[9px] text-zinc-600">low</span>
              <input
                type="number"
                min="1"
                max="10"
                step="0.5"
                value={sigmaHigh}
                onChange={(e) => setSigmaHigh(parseFloat(e.target.value))}
                className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center"
              />
              <span className="text-[9px] text-zinc-600">high</span>
            </div>
          </div>

          {/* Align Checkbox */}
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={align}
              onChange={(e) => setAlign(e.target.checked)}
              className="w-3 h-3 accent-indigo-500"
            />
            Sub-pixel alignment (ZNCC)
          </label>
        </div>

        {/* Composition Options */}
        <div className="border-t border-zinc-800/50 pt-2 space-y-1.5">
          <div className="text-[10px] text-zinc-500 uppercase tracking-wider mb-1">
            Composition
          </div>

          {/* White Balance */}
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">WB</label>
            <select
              value={wbMode}
              onChange={(e) => setWbMode(e.target.value)}
              className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
            >
              <option value="auto">Auto (Median)</option>
              <option value="none">None</option>
            </select>
          </div>

          {/* SCNR */}
          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={scnrEnabled}
              onChange={(e) => setScnrEnabled(e.target.checked)}
              className="w-3 h-3 accent-pink-500"
            />
            SCNR (Green Removal)
          </label>

          {scnrEnabled && (
            <div className="pl-4 space-y-1">
              <div className="flex items-center gap-2">
                <select
                  value={scnrMethod}
                  onChange={(e) => setScnrMethod(e.target.value)}
                  className="bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
                >
                  <option value="average">Average Neutral</option>
                  <option value="maximum">Maximum Neutral</option>
                </select>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.1"
                  value={scnrAmount}
                  onChange={(e) => setScnrAmount(parseFloat(e.target.value))}
                  className="flex-1 h-1 accent-pink-500"
                />
                <span className="text-[10px] text-zinc-300 font-mono w-6">
                  {(scnrAmount * 100).toFixed(0)}%
                </span>
              </div>
            </div>
          )}
        </div>

        {/* Output Info */}
        {estimatedOutputRes && (
          <div className="flex items-center gap-1.5 text-[10px] text-zinc-500">
            <Maximize2 size={9} />
            Output: {estimatedOutputRes}
          </div>
        )}

        {/* Processing State */}
        {isLoading ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 text-[11px] text-indigo-300">
                <Loader2 size={12} className="animate-spin" />
                {progressStage || `Processing ${totalFrames} frames…`}
              </div>
              <span className="text-[10px] text-zinc-500 font-mono">{elapsed}s</span>
            </div>
            <ProgressBar value={progress} variant="blue" indeterminate={progress <= 0} />
          </div>
        ) : (
          <button
            onClick={handleDrizzle}
            disabled={!canDrizzle}
            className="w-full flex items-center justify-center gap-2 bg-gradient-to-r from-indigo-600/20 to-pink-600/20 hover:from-indigo-600/30 hover:to-pink-600/30 text-indigo-300 border border-indigo-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            <Layers size={12} />
            Drizzle RGB ({scale}×)
          </button>
        )}

        {/* Validation Warning */}
        {!canDrizzle && !isLoading && totalFrames > 0 && (
          <div className="flex items-center gap-1.5 text-[10px] text-amber-400/70">
            <Info size={9} />
            Requires at least 2 channels with 2+ frames each
          </div>
        )}

        {/* Results */}
        {result && !isLoading && (
          <div className="space-y-1.5 border-t border-zinc-800/50 pt-2">
            {result.previewUrl && (
              <img
                src={result.previewUrl}
                alt="Drizzle RGB result"
                className="w-full rounded border border-zinc-700"
              />
            )}

            <div className="grid grid-cols-3 gap-1 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-red-400">R frames</div>
                <div className="text-zinc-300 font-mono">{result.frame_count_r || 0}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-green-400">G frames</div>
                <div className="text-zinc-300 font-mono">{result.frame_count_g || 0}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-blue-400">B frames</div>
                <div className="text-zinc-300 font-mono">{result.frame_count_b || 0}</div>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-1 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Input</div>
                <div className="text-zinc-300 font-mono">
                  {result.input_dims?.[0]}×{result.input_dims?.[1]}
                </div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-indigo-400">Output</div>
                <div className="text-zinc-300 font-mono">
                  {result.output_dims?.[0]}×{result.output_dims?.[1]}
                </div>
              </div>
            </div>

            <div className="text-[10px] text-zinc-500">
              {result.elapsed_ms} ms · {result.scale || scale}× scale · {result.rejected_pixels?.toLocaleString() || 0} rejected
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
