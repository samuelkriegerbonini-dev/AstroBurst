import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { hooks } from "../../../test/hookRuntime";
import { COLORBAR_HEIGHT_PX } from "../../../utils/colorbar";
import { LUT_WITH_NODATA_BYTES, STRETCH_KIND, type DisplayTransfer } from "../../../utils/displayTransfer";

vi.mock("react", async (importOriginal) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await importOriginal());
});

import Colorbar from "../Colorbar";

interface ColorbarProps {
  transfer: DisplayTransfer;
  lut: Uint8Array;
  unit: string | null;
  pending: boolean;
  centre: number | null;
}

interface PointerEventStub {
  clientX: number;
  currentTarget: { getBoundingClientRect: () => { left: number; width: number } };
}

interface StripProps {
  className?: string;
  style?: Record<string, unknown>;
  children?: ReactNode;
  onMouseMove?: (event: PointerEventStub) => void;
  onMouseLeave?: () => void;
  [data: `data-${string}`]: unknown;
}

const LINEAR: DisplayTransfer = {
  vmin: 0,
  vmax: 1000,
  shadow: 0,
  midtone: 0.5,
  highlight: 1,
  stretchKind: STRETCH_KIND.linear,
  asinhA: 0.1,
  power: 1,
  invert: false,
};

const LUT = new Uint8Array(LUT_WITH_NODATA_BYTES);

const ColorbarView = (Colorbar as unknown as { type: (props: ColorbarProps) => ReactNode }).type;

function draw(unit: string | null): ReactNode {
  return hooks.render(ColorbarView, { transfer: LINEAR, lut: LUT, unit, pending: false, centre: null }) as ReactNode;
}

function findData(node: ReactNode, attribute: `data-${string}`): StripProps | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findData(child, attribute);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<StripProps>(node)) return null;
  if (node.props[attribute] !== undefined) return node.props;
  return findData(node.props.children, attribute);
}

function hover(unit: string | null, clientX: number): void {
  const strip = findData(draw(unit), "data-colorbar-strip");
  strip?.onMouseMove?.({ clientX, currentTarget: { getBoundingClientRect: () => ({ left: 10, width: 200 }) } });
}

beforeEach(() => {
  hooks.mount();
});

afterEach(() => {
  hooks.mount();
});

describe("Colorbar hover marker", () => {
  it("draws a 1px marker over the strip at the hovered fraction, ignoring the pointer", () => {
    hover("MJy/sr", 60);
    const marker = findData(draw("MJy/sr"), "data-colorbar-marker");
    expect(marker).not.toBeNull();
    expect(marker?.style).toEqual({ left: "25%", height: COLORBAR_HEIGHT_PX });
    for (const utility of ["absolute", "top-0", "w-px", "pointer-events-none"]) {
      expect(marker?.className?.split(" ")).toContain(utility);
    }
  });

  it("draws no marker until the pointer is on the strip and removes it when the pointer leaves", () => {
    expect(findData(draw("MJy/sr"), "data-colorbar-marker")).toBeNull();
    hover("MJy/sr", 210);
    expect(findData(draw("MJy/sr"), "data-colorbar-marker")?.style?.left).toBe("100%");
    findData(draw("MJy/sr"), "data-colorbar-strip")?.onMouseLeave?.();
    expect(findData(draw("MJy/sr"), "data-colorbar-marker")).toBeNull();
  });
});

describe("Colorbar readback", () => {
  it("reads back the value and the unit together while hovering", () => {
    hover("MJy/sr", 60);
    const html = renderToStaticMarkup(draw("MJy/sr") as ReactElement);
    expect(html).toMatch(/data-colorbar-readback="[^"]*"[^>]*>250\.0 MJy\/sr<\/span>/);
    expect(html).not.toContain("data-colorbar-unit");
  });

  it("shows the unit alone at rest and the bare value for data without a unit", () => {
    expect(renderToStaticMarkup(draw("MJy/sr") as ReactElement)).toMatch(/data-colorbar-unit="[^"]*"[^>]*>MJy\/sr<\/span>/);
    hooks.mount();
    hover(null, 60);
    const html = renderToStaticMarkup(draw(null) as ReactElement);
    expect(html).toMatch(/data-colorbar-readback="[^"]*"[^>]*>250\.0<\/span>/);
    expect(html).not.toContain("null");
  });
});
