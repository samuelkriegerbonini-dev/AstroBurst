import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ExportNotice, { SAVED_NOTICE_MS } from "../ExportNotice";

describe("ExportNotice", () => {
  it("shows the whole saved path, a copy mark or an announced failure", () => {
    const path = "C:/Users/someone/Documents/observations/2026/jwst/nirspec/g235h/jw01266_spectra.csv";
    const savedPath = `${path}${"x".repeat(90 - path.length)}`;
    expect(savedPath).toHaveLength(90);
    const saved = renderToStaticMarkup(createElement(ExportNotice, { saved: savedPath }));
    expect(saved).toContain("data-export-saved");
    expect(saved).toContain(`Saved ${savedPath}`);
    expect(saved).toContain("break-all");
    expect(saved).not.toContain("truncate");
    const copied = renderToStaticMarkup(createElement(ExportNotice, { copied: true }));
    expect(copied).toContain("data-export-copied");
    expect(copied).toContain("copied");
    const failed = renderToStaticMarkup(createElement(ExportNotice, { error: "export failed: disk full" }));
    expect(failed).toContain('role="alert"');
    expect(failed).toContain("export failed: disk full");
    expect(renderToStaticMarkup(createElement(ExportNotice, {}))).toBe("");
    expect(SAVED_NOTICE_MS).toBe(6000);
  });
});
