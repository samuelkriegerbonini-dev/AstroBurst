import { isValidElement, type ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import CubePrepStepEditor from "../CubePrepStepEditor";
import { createCubePrepDraft } from "../cubePrepModel";

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
});
