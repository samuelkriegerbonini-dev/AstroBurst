import { describe, expect, it } from "vitest";
import { regionToolHint, toolAfterCommit } from "../regionToolHints";
import type { RegionShapeKind, RegionTool } from "../../shared/types/regions";

describe("regionToolHint", () => {
  it.each<[Exclude<RegionTool, "none">, string]>([
    ["select", "Click a region to select, drag to move; Delete removes; Esc deselects"],
    ["circle", "Drag from the center out to the radius"],
    ["ellipse", "Drag from the center; horizontal and vertical distances set the radii"],
    ["box", "Drag from one corner to the opposite corner"],
    ["annulus", "Drag from the center to the outer radius; the inner radius starts at half and has its own handle"],
    ["polygon", "Click each vertex; double-click or Enter closes; Esc cancels"],
    ["line", "Drag from start to end; Esc cancels"],
    ["point", "Click to place a point; the tool stays on for the next one. Delete removes the last point, Esc deselects it, Esc again turns the tool off"],
  ])("tells how to use the %s tool", (tool, hint) => {
    expect(regionToolHint(tool)).toBe(hint);
  });

  it("gives no hint while no tool is active", () => {
    expect(regionToolHint("none")).toBeNull();
  });
});

describe("toolAfterCommit", () => {
  it("keeps the Point tool and returns the other shapes to Select", () => {
    expect(toolAfterCommit("point")).toBe("point");
    const others: RegionShapeKind[] = ["circle", "ellipse", "box", "annulus", "polygon", "line"];
    for (const shape of others) expect(toolAfterCommit(shape)).toBe("select");
  });
});
