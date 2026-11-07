import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import CubePrepStepEditor from "../CubePrepStepEditor";
import { createCubePrepDraft, recipeProblems, setRecipeStep } from "../cubePrepModel";
import type { CubePrepStep } from "../../../shared/types/cubeprep";

type FieldProps = { label?: string; value?: unknown; onChange?: (event: { target: { value: string } }) => void; children?: ReactNode };
function field(node: ReactNode, label: string): FieldProps | undefined {
  if (Array.isArray(node)) { for (const child of node) { const hit = field(child, label); if (hit) return hit; } return undefined; }
  if (!isValidElement<FieldProps>(node)) return undefined;
  if (node.props.label === label && isValidElement<FieldProps>(node.props.children)) return node.props.children.props;
  return field(node.props.children, label);
}
const editor = (step: CubePrepStep, onChange = vi.fn()) => CubePrepStepEditor({ step, recipe: createCubePrepDraft().recipe, onChange, browse: vi.fn() });

function changeSelect(node: ReactNode, value: string, next: string): boolean {
  if (Array.isArray(node)) return node.some((child) => changeSelect(child, value, next));
  if (!isValidElement<{ value?: string; onChange?: (event: { target: { value: string } }) => void; children?: ReactNode }>(node)) return false;
  if (node.type === "select" && node.props.value === value) { node.props.onChange?.({ target: { value: next } }); return true; }
  return changeSelect(node.props.children, value, next);
}

describe("Cube Prep step editor", () => {
  it("removes fields of the previous crop mode before validation", () => {
    const onChange = vi.fn();
    const tree = CubePrepStepEditor({ step: { step: "crop", mode: "radec", ra: 53, dec: -27, size_arcsec: 2, rounding: "lightstack" }, recipe: createCubePrepDraft().recipe, onChange, browse: vi.fn() });
    changeSelect(tree, "radec", "common");
    expect(onChange).toHaveBeenCalledWith({ step: "crop", mode: "common", rounding: "lightstack" });
  });

  it("removes stale edit parameters when selecting a different cube operation", () => {
    const onChange = vi.fn();
    const tree = CubePrepStepEditor({ step: { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.1, use_dq: true }, recipe: createCubePrepDraft().recipe, onChange, browse: vi.fn() });
    changeSelect(tree, "drop_invalid", "header");
    expect(onChange).toHaveBeenCalledWith({ step: "edit", action: "header", redshift: null, ra_center: null, dec_center: null, use_brightest_pixel: false });
  });

  it("names the DQ mask that drop_invalid excludes for each dataset", () => {
    const html = renderToStaticMarkup(createElement(CubePrepStepEditor, { step: { step: "edit", action: "drop_invalid", threshold: 0, frac_valid: 0.01, use_dq: true }, recipe: createCubePrepDraft().recipe, onChange: vi.fn(), browse: vi.fn() }));
    expect(html).toContain("DQ exclusion uses the dataset mask: DO_NOT_USE (bit 0) for JWST, Roman and any other dataset; 0x3FFF (REED_SOLOMON..COSMIC_RAY_SINGLE) for HST.");
    expect(html).not.toContain("bit 0 only");
    expect(html).not.toContain("other HST flags still count");
  });

  it("supplies exactly the parameters accepted by the selected taper window", () => {
    const onChange = vi.fn();
    const tree = CubePrepStepEditor({ step: { step: "kernels", method: "window", reference: "F444W", window: "split_cosine_bell", alpha: 0.4, beta: 0.5, regularization: 0.001 }, recipe: createCubePrepDraft().recipe, onChange, browse: vi.fn() });
    changeSelect(tree, "split_cosine_bell", "hanning");
    expect(onChange).toHaveBeenCalledWith({ step: "kernels", method: "window", reference: "F444W", window: "hanning", regularization: 0.001, auto: false });
  });

  it("writes a real window default when switching kernel methods", () => {
    const onChange = vi.fn();
    const tree = CubePrepStepEditor({ step: { step: "kernels", reference: "F444W", method: "wiener", eps: 0.001 }, recipe: createCubePrepDraft().recipe, onChange, browse: vi.fn() });
    expect(changeSelect(tree, "wiener", "window")).toBe(true);
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({ method: "window", window: "hanning", regularization: 0.0001 }));
  });

  it("stores null for a cleared number, shows it empty and lets the recipe check ask for it (SCIENCE-12)", () => {
    const align: CubePrepStep = { step: "align", reference: "F444W", method: "bilinear", border_crop: 1, area_rule: "lightstack" };
    const onChange = vi.fn();
    field(editor(align, onChange), "Remove border pixels")!.onChange!({ target: { value: "" } });
    const cleared = onChange.mock.calls[0][0] as CubePrepStep;
    expect(cleared).toEqual({ ...align, border_crop: null });
    expect(field(editor(cleared), "Remove border pixels")!.value).toBe("");
    expect(field(editor({ step: "align", reference: "F444W", method: "bilinear", area_rule: "lightstack" }), "Remove border pixels")!.value).toBe(1);
    const recipe = { ...createCubePrepDraft().recipe, inputs: [{ path: "a.fits", dataset: "JWST", filter: "F444W" }], steps: [{ step: "cube" as const, output: "cube.fits" }] };
    expect(recipeProblems(setRecipeStep(recipe, cleared))).toEqual(["Align: enter a value for border_crop."]);
    field(editor(align, onChange), "Remove border pixels")!.onChange!({ target: { value: "2" } });
    expect(onChange).toHaveBeenLastCalledWith({ ...align, border_crop: 2 });
  });

  it("clears an optional number to null and keeps its box empty (SCIENCE-12)", () => {
    const header: CubePrepStep = { step: "edit", action: "header", redshift: 0.3, ra_center: null, dec_center: null, use_brightest_pixel: false };
    const onChange = vi.fn();
    field(editor(header, onChange), "Redshift (optional)")!.onChange!({ target: { value: "" } });
    expect(onChange).toHaveBeenCalledWith({ ...header, redshift: null });
    expect(field(editor(header), "Centre RA (degrees, optional)")!.value).toBe("");
    const empirical: CubePrepStep = { step: "psfs", source: "empirical", dataset: "JWST", filters: ["F444W"], data_dir: "", noise_threshold: null };
    expect(field(editor(empirical), "Noise threshold (optional)")!.value).toBe("");
    expect(field(editor({ step: "psfs", source: "empirical", dataset: "JWST", filters: ["F444W"], data_dir: "" }), "Noise threshold (optional)")!.value).toBe(0.5);
  });

  it("names every Browse button after its field (SCIENCE-25)", () => {
    const recipe = { ...createCubePrepDraft().recipe, inputs: [{ path: "a.fits", dataset: "JWST", filter: "F200W" }, { path: "b.fits", dataset: "JWST", filter: "" }] };
    const render = (step: CubePrepStep) => renderToStaticMarkup(createElement(CubePrepStepEditor, { step, recipe, onChange: vi.fn(), browse: vi.fn() }));
    expect(render({ step: "crop", mode: "region", region_file: "", index: 0, rounding: "lightstack" })).toContain('aria-label="Browse DS9 region file"');
    const files = render({ step: "psfs", source: "files", reference: "", files: [] });
    expect(files).toContain('aria-label="Browse F200W PSF"');
    expect(files).toContain('aria-label="Browse unspecified filter PSF"');
    expect(render({ step: "psfs", source: "empirical", dataset: "JWST", filters: ["F200W"], data_dir: "" })).toContain('aria-label="Browse star images and DS9 folder"');
  });
});
