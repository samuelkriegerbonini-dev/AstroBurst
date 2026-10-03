import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../../hooks/useMeasurementLog", () => ({ useMeasurementLog: () => [{}, {}, {}] }));

import AnalysisTabBar from "../AnalysisTabBar";
import AnalysisLogFooter from "../AnalysisLogFooter";
import { CUBE_TAB_UNAVAILABLE_TITLE } from "../../../utils/analysisSections";

type Attrs = Record<string, string>;

function openingTags(html: string, tag: string): Attrs[] {
  return [...html.matchAll(new RegExp(`<${tag}(\\s[^>]*)?>`, "g"))].map((m) => {
    const attrs: Attrs = {};
    for (const a of (m[1] ?? "").matchAll(/([\w-]+)(?:="([^"]*)")?/g)) attrs[a[1]] = a[2] ?? "";
    return attrs;
  });
}

function tabBarHtml(): string {
  return renderToStaticMarkup(
    createElement(AnalysisTabBar, {
      active: "sources",
      available: { image: true, sources: true, cube: false },
      onSelect: () => {},
    }),
  );
}

describe("AnalysisTabBar DOM contract", () => {
  it("renders a labelled tablist with one tab per group", () => {
    const html = tabBarHtml();
    const [list] = openingTags(html, "div");
    expect(list.role).toBe("tablist");
    expect(list["aria-label"]).toBe("Analysis groups");
    const tabs = openingTags(html, "button");
    expect(tabs.map((t) => t.role)).toEqual(["tab", "tab", "tab"]);
    expect(tabs.map((t) => t.id)).toEqual(["analysis-tab-image", "analysis-tab-sources", "analysis-tab-cube"]);
    expect(tabs.map((t) => t["data-analysis-tab"])).toEqual(["image", "sources", "cube"]);
    expect(tabs.map((t) => t["aria-controls"])).toEqual([
      "analysis-panel-image",
      "analysis-panel-sources",
      "analysis-panel-cube",
    ]);
    expect([...html.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map((m) => m[1])).toEqual(["Image", "Sources", "Cube"]);
  });

  it("marks only the active tab selected and focusable", () => {
    const tabs = openingTags(tabBarHtml(), "button");
    expect(tabs.map((t) => t["aria-selected"])).toEqual(["false", "true", "false"]);
    expect(tabs.map((t) => t.tabindex)).toEqual(["-1", "0", "-1"]);
  });

  it("disables only the cube tab, with an explanatory title", () => {
    const [image, sources, cube] = openingTags(tabBarHtml(), "button");
    expect(cube["aria-disabled"]).toBe("true");
    expect(cube.title).toBe(CUBE_TAB_UNAVAILABLE_TITLE);
    for (const t of [image, sources]) {
      expect(t["aria-disabled"]).toBeUndefined();
      expect(t.title).toBeUndefined();
    }
  });
});

describe("AnalysisLogFooter DOM contract", () => {
  it("renders a collapsed toggle with the entry count controlling the log body", () => {
    const html = renderToStaticMarkup(
      createElement(AnalysisLogFooter, null, createElement("section", { id: "analysis-log" })),
    );
    const divs = openingTags(html, "div");
    expect("data-analysis-log-footer" in divs[0]).toBe(true);
    const [toggle] = openingTags(html, "button");
    expect(toggle["aria-expanded"]).toBe("false");
    expect(toggle["aria-controls"]).toBe("analysis-log-body");
    expect(html).toContain("Measurement log (3)</button>");
    const body = divs.find((d) => d.id === "analysis-log-body");
    expect(body && "hidden" in body).toBe(true);
    expect(html).toMatch(/<div id="analysis-log-body"[^>]*><section id="analysis-log">/);
  });
});
