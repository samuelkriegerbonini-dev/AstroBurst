import { describe, it, expect } from "vitest";
import {
  EXAMPLE_EXPRESSIONS,
  MAX_SLOTS,
  RESERVED_NAMES,
  TARGET_SYMBOL,
  caretLines,
  isValidSlotIdentifier,
  nextSlotName,
  slotErrors,
  validateSlotName,
} from "../pixelmathSlots";

describe("isValidSlotIdentifier", () => {
  it("accepts identifiers starting with a letter or underscore", () => {
    for (const name of ["A", "ha", "_x1", "oiii_2", "Z9"]) {
      expect(isValidSlotIdentifier(name)).toBe(true);
    }
  });

  it("rejects digits first, punctuation, spaces and non-ascii", () => {
    for (const name of ["1A", "a-b", "a b", "", "é", "$T", "a.b"]) {
      expect(isValidSlotIdentifier(name)).toBe(false);
    }
  });
});

describe("validateSlotName", () => {
  it("returns null for a valid unused name and trims whitespace", () => {
    expect(validateSlotName("B", ["A"])).toBeNull();
    expect(validateSlotName(" B ", ["A"])).toBeNull();
  });

  it("rejects the reserved target symbol and any $-prefixed name", () => {
    expect(validateSlotName(TARGET_SYMBOL, [])).toMatch(/reserved/);
    expect(validateSlotName("$X", [])).toMatch(/reserved/);
  });

  it("rejects empty names and invalid identifiers", () => {
    expect(validateSlotName("", [])).toMatch(/required/);
    expect(validateSlotName("   ", [])).toMatch(/required/);
    expect(validateSlotName("1A", [])).toMatch(/letters, digits and underscores/);
    expect(validateSlotName("a-b", [])).toMatch(/letters, digits and underscores/);
  });

  it("rejects function names so they stay callable", () => {
    for (const name of ["med", "min", "e", "pi", "iif"]) {
      expect(validateSlotName(name, [])).toMatch(/function name/);
    }
    expect(RESERVED_NAMES).toContain("rescale");
  });

  it("rejects duplicates case-sensitively", () => {
    expect(validateSlotName("A", ["A", "B"])).toMatch(/already used/);
    expect(validateSlotName("a", ["A"])).toBeNull();
  });
});

describe("nextSlotName", () => {
  it("walks the alphabet skipping used letters", () => {
    expect(nextSlotName([])).toBe("A");
    expect(nextSlotName(["A", "B"])).toBe("C");
    expect(nextSlotName(["B"])).toBe("A");
  });

  it("falls back to numbered names once the alphabet is used", () => {
    const all = Array.from({ length: MAX_SLOTS }, (_, i) => String.fromCharCode(65 + i));
    expect(nextSlotName(all)).toBe("I1");
    expect(nextSlotName([...all, "I1"])).toBe("I2");
  });
});

describe("slotErrors", () => {
  it("flags every duplicate row and leaves valid rows null", () => {
    expect(slotErrors([{ name: "A" }, { name: "A" }])).toEqual([
      "'A' is already used",
      "'A' is already used",
    ]);
    expect(slotErrors([{ name: "A" }, { name: "B" }])).toEqual([null, null]);
    expect(slotErrors([{ name: "A " }, { name: " A" }])[0]).toMatch(/already used/);
  });
});

describe("caretLines", () => {
  it("places the caret under the failing token on a single line", () => {
    expect(caretLines("1 + * 2", 4, 1)).toEqual({ line: "1 + * 2", marker: "    ^" });
  });

  it("selects the line containing the position in a multi-line expression", () => {
    expect(caretLines("1 +\n* 2", 4, 1)).toEqual({ line: "* 2", marker: "^" });
    expect(caretLines("a\nbb\nfoo(1)", 5, 3)).toEqual({ line: "foo(1)", marker: "^^^" });
  });

  it("marks the end of the expression when the error is at the end", () => {
    expect(caretLines("1 +", 3, 1)).toEqual({ line: "1 +", marker: "   ^" });
    expect(caretLines("", 0, 0)).toEqual({ line: "", marker: "^" });
  });

  it("clips the marker to the current line", () => {
    expect(caretLines("ab\ncd", 1, 10)).toEqual({ line: "ab", marker: " ^" });
    expect(caretLines("abc", 10, 1)).toEqual({ line: "abc", marker: "   ^" });
  });
});

describe("EXAMPLE_EXPRESSIONS", () => {
  it("contains the documented starter expressions", () => {
    const expressions = EXAMPLE_EXPRESSIONS.map((e) => e.expression);
    expect(expressions).toEqual([
      "$T - med($T)",
      "iif($T != $T, 0, $T)",
      "(A + B + C) / 3",
      "~$T",
      "rescale($T, min($T), max($T), 0, 1)",
      "$T * (A / med(A))",
    ]);
  });
});

import { autoSlotsFromFiles, bindMissingSlots, missingSlots, referencedSymbols } from "../pixelmathSlots";

describe("automatic slot binding", () => {
  const files = [{ path: "C:/d/target.fits" }, { path: "C:/d/a.fits" }, { path: "C:/d/b.fits" }, { path: "C:/d/c.fits" }];

  it("binds every loaded file except the target to the next free letters", () => {
    const slots = autoSlotsFromFiles(files, "C:/d/target.fits", []);
    expect(slots).toEqual([
      { name: "A", path: "C:/d/a.fits" },
      { name: "B", path: "C:/d/b.fits" },
      { name: "C", path: "C:/d/c.fits" },
    ]);
  });

  it("skips paths that are already bound and continues the letter sequence", () => {
    const slots = autoSlotsFromFiles(files, null, [{ name: "A", path: "C:/d/a.fits" }]);
    expect(slots.map((s) => s.name)).toEqual(["B", "C", "D"]);
    expect(slots.map((s) => s.path)).toEqual(["C:/d/target.fits", "C:/d/b.fits", "C:/d/c.fits"]);
  });

  it("caps the total number of slots", () => {
    const many = Array.from({ length: MAX_SLOTS + 5 }, (_, i) => ({ path: `C:/d/f${i}.fits` }));
    expect(autoSlotsFromFiles(many, null, []).length).toBe(MAX_SLOTS);
  });

  it("lists referenced image symbols without functions, the target or reserved names", () => {
    expect(referencedSymbols("(A + B + C) / 3")).toEqual(["A", "B", "C"]);
    expect(referencedSymbols("$T * (A / med(A))")).toEqual(["A"]);
    expect(referencedSymbols("iif($T != $T, 0, $T)")).toEqual([]);
    expect(referencedSymbols("rescale($T, min($T), max($T), 0, 1)")).toEqual([]);
    expect(referencedSymbols("dark_frame - flat")).toEqual(["dark_frame", "flat"]);
    expect(referencedSymbols("$T * 1e-3 + 2E5 - 1.5e2")).toEqual([]);
    expect(referencedSymbols("A2 + B_1")).toEqual(["A2", "B_1"]);
  });

  it("reports only the symbols that have no slot", () => {
    expect(missingSlots("(A + B + C) / 3", ["A"])).toEqual(["B", "C"]);
    expect(missingSlots("~$T", [])).toEqual([]);
  });

  it("binds missing symbols to unbound loaded files and falls back to the target", () => {
    const added = bindMissingSlots("(A + B + C + D) / 4", files, "C:/d/target.fits", [{ name: "A", path: "C:/d/a.fits" }]);
    expect(added).toEqual([
      { name: "B", path: "C:/d/b.fits" },
      { name: "C", path: "C:/d/c.fits" },
      { name: "D", path: "C:/d/target.fits" },
    ]);
  });

  it("binds nothing when there is neither a free file nor a target", () => {
    expect(bindMissingSlots("(A + B) / 2", [], null, [])).toEqual([]);
  });

  it("never binds a name that is not a valid slot identifier", () => {
    expect(bindMissingSlots("$T + pi", files, null, [])).toEqual([]);
  });
});
