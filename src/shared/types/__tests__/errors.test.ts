import { describe, expect, it } from "vitest";
import { TauriCommandError, normalizeTauriError } from "../errors";

describe("normalizeTauriError", () => {
  it("keeps the code and copies a plain-object context from a structured rejection", () => {
    const err = normalizeTauriError("mast_search_cmd", {
      code: "mast_rate_limited",
      message: "too many requests",
      context: { retryAfterSec: 42, consecutive: 1, status: 429, operation: "search/jwst/api/v0.1/search" },
    });
    expect(err).toBeInstanceOf(TauriCommandError);
    expect(err.command).toBe("mast_search_cmd");
    expect(err.message).toBe("too many requests");
    expect(err.code).toBe("mast_rate_limited");
    expect(err.context).toEqual({ retryAfterSec: 42, consecutive: 1, status: 429, operation: "search/jwst/api/v0.1/search" });
  });

  it("keeps an empty context object", () => {
    const err = normalizeTauriError("mast_cancel_cmd", { code: "mast_cancelled", message: "cancelled", context: {} });
    expect(err.context).toEqual({});
  });

  it.each([
    ["a string", "status 429"],
    ["an array", [1, 2]],
    ["null", null],
    ["a number", 7],
  ])("ignores a context that is %s", (_label, context) => {
    const err = normalizeTauriError("mast_search_cmd", { code: "mast_server", message: "bad gateway", context });
    expect(err.code).toBe("mast_server");
    expect(err.context).toBeUndefined();
  });

  it("does not copy a context built from a class instance", () => {
    const err = normalizeTauriError("mast_search_cmd", { code: "mast_server", message: "x", context: new Date(0) });
    expect(err.context).toBeUndefined();
  });

  it("keeps string errors unchanged, without a code or context", () => {
    const err = normalizeTauriError("get_config", "config file is locked");
    expect(err.message).toBe("config file is locked");
    expect(err.code).toBeUndefined();
    expect(err.context).toBeUndefined();
  });

  it("returns an existing TauriCommandError as it is", () => {
    const original = new TauriCommandError("mast_search_cmd", "m", "mast_timeout", { timeoutSec: 120 });
    expect(normalizeTauriError("other", original)).toBe(original);
  });
});
