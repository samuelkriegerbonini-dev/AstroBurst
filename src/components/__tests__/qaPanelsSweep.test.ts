import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import profilesSource from "../regions/RegionProfilesPanel.tsx?raw";
import { Segmented } from "../regions/RegionProfilesPanel";
import { PREP_FIELD } from "../cubeprep/CubePrepStepEditor";
import { adHocErrorBoxLines, localSmallButtonLines } from "../../test/panelRules";

const RADIUS_UNITS = ["px", "arcsec"] as const;

function buttonOf(html: string, text: string): string {
  const chunk = html.split("<button").find((part) => part.includes(`>${text}</button>`));
  expect(chunk, `${text} button`).toBeDefined();
  return chunk!.slice(0, chunk!.indexOf(">"));
}

function occurrences(source: string, needle: string): number {
  return source.split(needle).length - 1;
}

describe("profiles panel sweep", () => {
  it("keeps no hand-made failure box and no private small button class in the profiles panel", () => {
    expect(adHocErrorBoxLines(profilesSource)).toEqual([]);
    expect(localSmallButtonLines(profilesSource)).toEqual([]);
    expect(profilesSource).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });

  it("tells which segment is active without relying on colour", () => {
    const html = renderToStaticMarkup(
      createElement(Segmented, { options: RADIUS_UNITS, value: "px", labels: (u: string) => u, label: "Radius unit", onChange: () => {} }),
    );
    expect(html).toContain('role="group"');
    expect(html).toContain('aria-label="Radius unit"');
    expect(occurrences(html, 'aria-pressed="true"')).toBe(1);
    expect(buttonOf(html, "px")).toContain('aria-pressed="true"');
    expect(buttonOf(html, "arcsec")).toContain('aria-pressed="false"');
  });

  it("keeps a disabled option disabled and unpressed", () => {
    const html = renderToStaticMarkup(
      createElement(Segmented, {
        options: RADIUS_UNITS,
        value: "px",
        labels: (u: string) => u,
        disabledOption: (u: string) => u === "arcsec",
        onChange: () => {},
      }),
    );
    const arcsec = buttonOf(html, "arcsec");
    expect(arcsec).toContain('disabled=""');
    expect(arcsec).toContain('aria-pressed="false"');
    expect(html.slice(0, html.indexOf("<button"))).not.toContain("role=");
  });

  it("names the three switches", () => {
    for (const label of ['label="Profile kind"', 'label="Radius unit"', 'label="Profile value"']) {
      expect(occurrences(profilesSource, label), label).toBe(1);
    }
  });

  it("hands the profile table to the shared copy button and reports a refused clipboard write", () => {
    expect(profilesSource).toContain("<CopyCsvButton");
    expect(profilesSource).not.toContain("navigator.clipboard");
    expect(profilesSource).toContain("onError={setCopyError}");
    expect(profilesSource).toContain("message={copyError}");
    expect(profilesSource).toContain("Copy every bin as CSV, with how the profile was measured");
    expect(profilesSource).toContain("Waiting for the profile that matches the current region and DQ setting");
  });
});

describe("Cube Prep fields sweep", () => {
  it("gives the Cube Prep fields a focus ring of their own", () => {
    expect(PREP_FIELD).toContain("focus:outline-none");
    expect(PREP_FIELD).toContain("focus-visible:ring-1");
    expect(PREP_FIELD).toContain("focus-visible:ring-cyan-500/60");
  });
});
