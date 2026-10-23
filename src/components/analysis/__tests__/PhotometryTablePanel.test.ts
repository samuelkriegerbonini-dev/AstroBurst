import { describe, expect, it } from "vitest";
import panelSource from "../PhotometryTablePanel.tsx?raw";

function bodyCellClasses(): { label: string[]; numeric: string[] } {
  const match = panelSource.match(/data-column=\{column\.key\}\s+className=\{`\$\{column\.key === "label" \? "([^"]*)" : "([^"]*)"\}/);
  expect(match).not.toBeNull();
  return { label: match![1].split(" "), numeric: match![2].split(" ") };
}

describe("PhotometryTablePanel cells (UX-07)", () => {
  it("routes the flux and AB cells through the shared cell formatters", () => {
    expect(panelSource).toMatch(/case "net_flux":\s*return fluxCellText\(p\.net_flux\);/);
    expect(panelSource).toMatch(/case "mag_ab":\s*return abCellText\(p\.mag_ab, p\.mag_ab_err\);/);
  });

  it("tags every body cell with its column key", () => {
    expect(panelSource).toMatch(/<td\s+key=\{column\.key\}\s+data-column=\{column\.key\}/);
  });

  it("keeps every body cell on one line, so an AB cell such as 24.512 ± 0.031 never wraps in the docked panel", () => {
    const classes = bodyCellClasses();
    expect(classes.label).toContain("truncate");
    expect(classes.numeric).toContain("whitespace-nowrap");
  });
});
