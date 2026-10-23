import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import OutputNote from "../OutputNote";
import { OUTPUT_FOLDER_WARNING } from "../../../utils/outputNote";

const PATH = "C:\\Users\\me\\AppData\\Roaming\\com.astroburst.desktop\\output\\cube_moment0_5-39.fits";

describe("OutputNote", () => {
  it("shows the file name, the cleared-at-launch warning and Show in folder, and carries the full path", () => {
    const html = renderToStaticMarkup(createElement(OutputNote, { path: PATH }));
    expect(html).toContain("data-output-note");
    expect(html).toContain(`data-output-path="${PATH}"`);
    expect(html).toContain(">cube_moment0_5-39.fits<");
    expect(html).toContain(OUTPUT_FOLDER_WARNING);
    expect(html).toContain("Show in folder");
    expect(html).not.toContain('role="alert"');
  });
});
