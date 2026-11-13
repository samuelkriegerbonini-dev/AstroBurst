import { describe, expect, it } from "vitest";
import source from "../ExportPanel.tsx?raw";

describe("ExportPanel Compress MEF source", () => {
  it("compresses the real file of a band plane", () => {
    expect(source).toContain("const sourcePath = sourceFile(filePath);");
    expect(source).toMatch(/import \{[^}]*\bsourceFile\b[^}]*\} from "\.\.\/\.\.\/utils\/imageRef";/);
    expect(source).not.toContain("parseImageRef(filePath)");
  });
});
