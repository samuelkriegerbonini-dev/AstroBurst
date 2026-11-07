import type { ReactNode } from "react";
import type { CubePrepRecipe, CubePrepStep } from "../../shared/types/cubeprep";

export const PREP_FIELD = "w-full min-w-0 bg-zinc-900 border border-zinc-700/60 rounded px-2 py-1 text-xs text-zinc-200 focus:outline-none focus:border-cyan-500/60 disabled:opacity-50";
export const PREP_BUTTON = "rounded border border-zinc-700 px-2 py-1 text-[11px] text-zinc-300 hover:bg-zinc-800 disabled:opacity-40";

export function PrepField({ label, children }: { label: string; children: ReactNode }) {
  return <label className="flex min-w-0 flex-col gap-1 text-[10px] text-zinc-400">{label}{children}</label>;
}

const sentence = (label: string) => /^[A-Z][a-z]/.test(label) ? `${label[0].toLowerCase()}${label.slice(1)}` : label;
const psfLabel = (filter: string) => `${filter || "Unspecified filter"} PSF`;

interface Props {
  step: CubePrepStep;
  recipe: CubePrepRecipe;
  onChange: (step: CubePrepStep) => void;
  browse: (directory: boolean, selected: (path: string) => void) => void;
}

export default function CubePrepStepEditor({ step, recipe, onChange, browse }: Props) {
  const patch = (change: Record<string, unknown>) => onChange({ ...step, ...change });
  const choose = (key: string, value: string) => {
    if (step.step === "crop" && key === "mode") {
      const fields = value === "reference" ? { reference: "" } : value === "region" ? { region_file: "", index: 0 } : value === "region_common" ? { region_file: "" } : {};
      onChange({ step: "crop", mode: value, rounding: step.rounding ?? "lightstack", ...fields });
    } else if (step.step === "edit" && key === "action") {
      const fields = value === "header" ? { redshift: null, ra_center: null, dec_center: null, use_brightest_pixel: false }
        : value === "drop_invalid" ? { threshold: 0, frac_valid: 0.01, use_dq: false }
        : value === "reference_nan_mask" ? { reference_slice: 0, mode: "reference" }
        : value === "remove_filter" ? { filter: "" }
        : value === "cut_sky" ? { rounding: "astroburst" } : {};
      onChange({ step: "edit", action: value, ...fields });
    } else if (step.step === "kernels" && key === "window") {
      const fields = value === "top_hat" ? { beta: 0.5 } : value === "split_cosine_bell" ? { alpha: 0.5, beta: 0.5 } : value === "hanning" ? {} : { alpha: 0.5 };
      onChange({ step: "kernels", method: "window", reference: step.reference, window: value, regularization: step.regularization ?? 0.0001, auto: false, ...fields });
    } else patch({ [key]: value });
  };
  const text = (key: string, label: string, optional = false) => <PrepField key={key} label={label}><input className={PREP_FIELD} value={String(step[key] ?? "")} onChange={(e) => patch({ [key]: optional && !e.target.value ? null : e.target.value })} /></PrepField>;
  const number = (key: string, label: string, fallback?: number) => <PrepField key={key} label={label}><input type="number" step="any" className={PREP_FIELD} value={typeof step[key] === "number" ? step[key] as number : key in step ? "" : fallback ?? ""} onChange={(e) => patch({ [key]: e.target.value === "" ? null : Number(e.target.value) })} /></PrepField>;
  const select = (key: string, label: string, options: readonly (string | [string, string])[], fallback = "") => <PrepField key={key} label={label}><select className={PREP_FIELD} value={String(step[key] ?? fallback)} onChange={(e) => choose(key, e.target.value)}>{options.map((option) => { const [value, name] = typeof option === "string" ? [option, option] : option; return <option key={value} value={value}>{name}</option>; })}</select></PrepField>;
  const toggle = (key: string, label: string, fallback = false) => <label key={key} className="flex items-center gap-2 text-[11px] text-zinc-300"><input type="checkbox" checked={Boolean(step[key] ?? fallback)} onChange={(e) => patch({ [key]: e.target.checked })} />{label}</label>;
  const path = (key: string, label: string, directory = false) => <div key={key} className="flex items-end gap-1"><div className="min-w-0 flex-1">{text(key, label)}</div><button type="button" className={PREP_BUTTON} aria-label={`Browse ${sentence(label)}`} onClick={() => browse(directory, (value) => patch({ [key]: value }))}>Browse</button></div>;
  const reference = select("reference", "Reference filter", [["", "Choose a reference"], ...recipe.inputs.map((input): [string, string] => [input.filter, input.filter])]);
  const sky = <div className="grid grid-cols-2 gap-2">{number("ra", "RA (degrees)")}{number("dec", "Dec (degrees)")}{number("size_arcsec", "Size (arcsec)")}</div>;
  const pixel = <div className="grid grid-cols-2 gap-2">{number("x_start", "X start (zero-based)")}{number("x_end", "X end (exclusive)")}{number("y_start", "Y start (zero-based)")}{number("y_end", "Y end (exclusive)")}</div>;
  let controls: ReactNode;
  switch (step.step) {
    case "units": controls = <>{select("to", "Output unit", ["Jy/pixel", "MJy/sr"])}<p>Conversion uses the input calibration and pixel scale. Missing calibration is an error.</p></>; break;
    case "crop": controls = <>
      {select("mode", "Crop region", [["common", "Common footprint"], ["radec", "Sky coordinates"], ["reference", "Reference footprint"], ["region", "DS9 region"], ["region_common", "DS9 common footprint"], ["pixel", "Pixel rectangle"]])}
      {step.mode === "radec" && sky}{step.mode === "reference" && reference}{step.mode === "pixel" && pixel}
      {(step.mode === "region" || step.mode === "region_common") && path("region_file", "DS9 region file")}
      {step.mode === "region" && number("index", "Region index (zero-based)", 0)}
      {select("rounding", "Crop rounding", [["lightstack", "LIGHTSTACK"], ["astroburst", "AstroBurst"]])}
    </>; break;
    case "align": controls = <>{reference}{select("method", "Resampling", ["bilinear", "bicubic", "exact"])}{number("border_crop", "Remove border pixels", 1)}{select("area_rule", "Pixel-area correction", [["lightstack", "LIGHTSTACK compatibility"], ["scale", "Scale per-pixel flux"], ["keep", "Keep surface brightness"]])}<p>Resampling introduces correlated errors. Inspect footprints and edge masks after alignment.</p></>; break;
    case "cube": controls = <>{reference}{select("order", "Band order", [["wavelength", "Increasing wavelength"], ["given", "Input order"]])}{select("layout", "FITS layout", [["lightstack", "LIGHTSTACK (SCI only)"], ["mef", "MEF (SCI / ERR / DQ when available)"]])}{text("output", "Cube filename")}</>; break;
    case "edit": controls = <>
      {select("action", "Cube operation", [["drop_invalid", "Drop invalid bands"], ["remove_filter", "Remove one filter"], ["cut", "Pixel crop"], ["cut_sky", "Sky crop"], ["header", "Set centre / redshift"], ["reference_nan_mask", "Apply common validity mask"]])}
      {step.action === "drop_invalid" && <>{number("threshold", "Signal threshold", 0)}{number("frac_valid", "Minimum valid fraction", 0.01)}{toggle("use_dq", "Exclude flagged DQ pixels")}<p>DQ exclusion uses the dataset mask: DO_NOT_USE (bit 0) for JWST, Roman and any other dataset; 0x3FFF (REED_SOLOMON..COSMIC_RAY_SINGLE) for HST.</p></>}
      {step.action === "remove_filter" && select("filter", "Filter to remove", recipe.inputs.map((input) => input.filter))}
      {step.action === "cut" && pixel}{step.action === "cut_sky" && <>{sky}{select("rounding", "Rounding", ["astroburst", "lightstack"], "astroburst")}</>}
      {step.action === "header" && <>{number("redshift", "Redshift (optional)")}{number("ra_center", "Centre RA (degrees, optional)")}{number("dec_center", "Centre Dec (degrees, optional)")}{toggle("use_brightest_pixel", "Use brightest pixel as centre")}</>}
      {step.action === "reference_nan_mask" && <>{number("reference_slice", "Reference band index", 0)}{select("mode", "Mask combination", ["reference", "union", "intersection"], "reference")}</>}
      <p>Repeated edits and custom step ordering can be entered in the recipe editor.</p>
    </>; break;
    case "psfs": {
      const files = Array.isArray(step.files) ? step.files as { filter: string; path: string }[] : [];
      const updateFile = (filter: string, value: string) => patch({ files: [...files.filter((file) => file.filter !== filter), { filter, path: value }] });
      controls = <>
        <PrepField label="PSF source"><select className={PREP_FIELD} value={String(step.source)} onChange={(e) => onChange(e.target.value === "files" ? { step: "psfs", source: "files", reference: step.reference, files: recipe.inputs.map((input) => ({ filter: input.filter, path: "" })) } : { step: "psfs", source: "empirical", reference: step.reference, dataset: recipe.inputs[0]?.dataset ?? "", filters: recipe.inputs.map((input) => input.filter), data_dir: "" })}><option value="files">Existing FITS PSFs</option><option value="empirical">Estimate from stars / DS9 boxes</option></select></PrepField>
        {reference}{number("pixel_scale_arcsec", "PSF pixel scale (arcsec, optional)")}
        {step.source === "files" && recipe.inputs.map((input) => <div key={input.filter} className="flex items-end gap-1"><div className="min-w-0 flex-1"><PrepField label={psfLabel(input.filter)}><input className={PREP_FIELD} value={files.find((file) => file.filter === input.filter)?.path ?? ""} onChange={(e) => updateFile(input.filter, e.target.value)} /></PrepField></div><button type="button" className={PREP_BUTTON} aria-label={`Browse ${sentence(psfLabel(input.filter))}`} onClick={() => browse(false, (value) => updateFile(input.filter, value))}>Browse</button></div>)}
        {step.source === "empirical" && <>
          {text("dataset", "PSF dataset")}{path("data_dir", "Star images and DS9 folder", true)}
          <p>Provide one FITS image per filter with the filter in its filename, and stars_&lt;filter in lowercase&gt;.reg DS9 boxes beside it. Filters: {Array.isArray(step.filters) ? step.filters.join(", ") : "none"}.</p>
          <details><summary className="cursor-pointer text-zinc-300">Star selection and cleaning</summary><div className="mt-2 grid gap-2">
            {number("cutout_size", "Star cutout size (pixels)", 61)}{number("psf_size", "Final PSF size (pixels)", 51)}{number("background_border", "Background border (pixels)", 10)}{number("background_sigma", "Background sigma", 4)}
            {number("noise_threshold", "Noise threshold (optional)", 0.5)}{toggle("clip_negative", "Clip negative values", true)}{toggle("radial_cleaning", "Radial cleaning")}{number("cleaning_sigma", "Cleaning sigma", 1)}{number("cleaning_consecutive_bins", "Consecutive cleaning bins", 3)}
            {number("normalization_radius", "Normalization radius", 20)}{select("normalization_radius_unit", "Radius unit", ["pixel", "arcsec"], "pixel")}{number("oversampling", "Oversampling (optional)")}{select("stack", "Star stack", ["median", "mean"], "median")}
          </div></details>
        </>}
      </>;
      break;
    }
    case "kernels": controls = <>
      {reference}<PrepField label="Kernel method"><select className={PREP_FIELD} value={String(step.method)} onChange={(e) => onChange(e.target.value === "window" ? { step: "kernels", reference: step.reference, method: "window", window: "hanning", regularization: 0.0001, auto: false } : { step: "kernels", reference: step.reference, method: "wiener", eps: 0.001, eps_mode: "absolute", shape: "reference", auto: false })}><option value="wiener">wiener</option><option value="window">window</option></select></PrepField>
      {step.method === "wiener" ? <>{number("eps", "Wiener regularization ε", 0.001)}{select("eps_mode", "Regularization scale", ["absolute", "relative"], "absolute")}</> : <>{select("window", "Taper window", ["hanning", "top_hat", "cosine_bell", "split_cosine_bell", "tukey"], "hanning")}{["cosine_bell", "split_cosine_bell", "tukey"].includes(String(step.window)) && number("alpha", "Window α", 0.5)}{["top_hat", "split_cosine_bell"].includes(String(step.window)) && number("beta", "Window β", 0.5)}{number("regularization", "Regularization", 0.0001)}</>}
      {toggle("auto", "Choose regularization / window parameter automatically")}<p>Choose the broadest PSF as reference. The report includes residuals, FWHM and encircled-energy diagnostics.</p>
    </>; break;
    case "match": controls = <>{reference}{select("mode", "PSF matching convention", [["lightstack", "LIGHTSTACK"], ["astroburst", "AstroBurst (propagate ERR / DQ)"]])}{toggle("allow_unmatched", "Allow bands without a matching kernel")}<p>Unmatched bands are not suitable for a common-PSF regional SED. Check every warning before using SAGUI.</p></>; break;
    case "export": controls = <>{text("output", "Export filename") }<p>The run folder also retains intermediate products, the effective recipe and the result manifest.</p></>; break;
  }
  return <div className="grid gap-3 text-[10px] text-zinc-500">{controls}</div>;
}
