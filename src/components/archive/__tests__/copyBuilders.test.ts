import { describe, expect, it } from "vitest";
import { coordinatesText, curlCommand, requestJson, rowTsv } from "../logic/copyBuilders";
import { COLUMNS, row } from "./fixtures";

const BODY = { target: ["274.688 -13.792"], radius: 3, radius_units: "arcminutes", conditions: [{ productLevel: "3" }], limit: 200, offset: 0 };

describe("requestJson", () => {
  it("prints the exact body MAST received, indented", () => {
    const text = requestJson(BODY);
    expect(JSON.parse(text)).toEqual(BODY);
    expect(text).toContain('"productLevel": "3"');
  });
});

describe("curlCommand", () => {
  it("posts the body to the mission search endpoint without any Authorization for open missions", () => {
    const curl = curlCommand("jwst", BODY);
    expect(curl.startsWith("curl -X POST https://mast.stsci.edu/search/jwst/api/v0.1/search")).toBe(true);
    expect(curl).toContain(`-H "Content-Type: application/json"`);
    expect(curl).toContain(`-d '${JSON.stringify(BODY)}'`);
    expect(curl).not.toMatch(/authorization/i);
  });

  it("adds the token placeholder, never a token, for Roman", () => {
    const curl = curlCommand("roman", BODY);
    expect(curl.startsWith("curl -X POST https://mast.stsci.edu/search/roman/api/v0.1/search")).toBe(true);
    expect(curl).toContain(`-H "Authorization: token $MAST_API_TOKEN"`);
  });

  it("escapes single quotes in the body for the shell", () => {
    const curl = curlCommand("hst", { target: ["Barnard's Star"] });
    expect(curl).toContain(`-d '{"target":["Barnard'\\''s Star"]}'`);
  });
});

describe("rowTsv", () => {
  it("copies a header line and a value line, tabs and newlines flattened", () => {
    const r = row("jw02739-o001_t001_nircam", {
      values: { fileSetName: "jw02739-o001_t001_nircam", productLevel: "3", effexptm: 5797.86, date_obs: "2022-08-14T08:33:45", program: 2739, _prog_obs: "a\tb\nc" },
    });
    expect(rowTsv(r, COLUMNS)).toBe(
      ["Dataset\tLvl\tExp (s)\tDate obs (UTC)\tProgram\tProg.Obs", "jw02739-o001_t001_nircam\t3\t5797.86\t2022-08-14T08:33:45\t2739\ta b c"].join("\n"),
    );
  });

  it("leaves missing values empty", () => {
    const r = row("x", { values: { fileSetName: "x" } });
    expect(rowTsv(r, COLUMNS.slice(0, 3)).split("\n")[1]).toBe("x\t\t");
  });
});

describe("coordinatesText", () => {
  it("prints five decimals", () => {
    expect(coordinatesText(274.688, -13.792)).toBe("274.68800 -13.79200");
  });
});
