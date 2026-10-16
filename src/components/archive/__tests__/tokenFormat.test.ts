import { describe, expect, it } from "vitest";
import { MAST_TOKEN_PAGE_URL, checkTokenFormat } from "../logic/tokenFormat";

describe("checkTokenFormat", () => {
  it("accepts the marker token and trims surrounding whitespace", () => {
    expect(checkTokenFormat("  AB-TEST-TOKEN-0123456789abcdef \n")).toEqual({ ok: true, token: "AB-TEST-TOKEN-0123456789abcdef" });
  });

  it("accepts 16 and 512 printable characters", () => {
    expect(checkTokenFormat("!".repeat(16)).ok).toBe(true);
    expect(checkTokenFormat("~".repeat(512)).ok).toBe(true);
  });

  it("refuses short, long and empty input", () => {
    expect(checkTokenFormat("")).toEqual({ ok: false, reason: "Paste the token you created on the MAST token page." });
    expect(checkTokenFormat("a".repeat(15))).toEqual({ ok: false, reason: "This is too short for a MAST token (at least 16 characters)." });
    expect(checkTokenFormat("a".repeat(513))).toEqual({ ok: false, reason: "This is too long for a MAST token (at most 512 characters)." });
  });

  it("refuses inner whitespace, control characters and non-ASCII", () => {
    const reason = "A MAST token has only printable ASCII characters and no spaces.";
    expect(checkTokenFormat("abcdefgh ijklmnopq")).toEqual({ ok: false, reason });
    expect(checkTokenFormat("abcdefghijklmnop\u0007")).toEqual({ ok: false, reason });
    expect(checkTokenFormat("abcdefghijklmnopé")).toEqual({ ok: false, reason });
  });
});

describe("MAST_TOKEN_PAGE_URL", () => {
  it("is the token page with the AstroBurst name and the exclusive-access scope", () => {
    expect(MAST_TOKEN_PAGE_URL).toBe("https://auth.mast.stsci.edu/token?suggested_name=AstroBurst&suggested_scope=mast:exclusive_access");
  });
});
