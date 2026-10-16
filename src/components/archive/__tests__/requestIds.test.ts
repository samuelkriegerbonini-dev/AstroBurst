import { describe, expect, it } from "vitest";
import {
  TOKEN_CHECK_ID,
  TOKEN_SAVE_ID,
  generationOf,
  generationPrefix,
  isValidRequestId,
  listId,
  pageId,
  previewId,
  productsId,
  resolveId,
  searchId,
  thumbId,
} from "../logic/requestIds";

describe("request ids", () => {
  it("formats every kind as g<N>:<kind>:<parts>", () => {
    expect(searchId(3)).toBe("g3:search");
    expect(resolveId(3)).toBe("g3:resolve");
    expect(pageId(3, 400)).toBe("g3:page:400");
    expect(listId(3, "jw02739-o001_t001_nircam")).toBe("g3:list:jw02739-o001_t001_nircam");
    expect(thumbId(3, "ICK909030", null)).toBe("g3:thumb:ICK909030");
    expect(thumbId(3, "ICK909030", "ick909030_drc_thumb.jpg")).toBe("g3:thumb:ICK909030:ick909030_drc_thumb.jpg");
    expect(previewId(3, "a", "b.jpg", "pane")).toBe("g3:preview:a:b.jpg:pane");
    expect(previewId(3, "a", "b.jpg", "large")).toBe("g3:preview:a:b.jpg:large");
    expect(productsId(3, "a")).toBe("g3:products:a");
  });

  it("uses fixed ids for the token commands", () => {
    expect(TOKEN_SAVE_ID).toBe("token:save");
    expect(TOKEN_CHECK_ID).toBe("token:check");
  });

  it("cancels a generation by its prefix, which never matches the next one", () => {
    expect(generationPrefix(1)).toBe("g1:");
    expect(searchId(12).startsWith(generationPrefix(1))).toBe(false);
    expect(pageId(1, 200).startsWith(generationPrefix(1))).toBe(true);
  });

  it("keeps the generation for page ids", () => {
    expect(generationOf(pageId(7, 1800))).toBe(7);
    expect(generationOf(searchId(7))).toBe(7);
    expect(generationOf(TOKEN_SAVE_ID)).toBeNull();
  });

  it("produces ids Rust accepts", () => {
    for (const id of [searchId(1), pageId(2, 200), previewId(9, "jw02739001002_02101_00001", "jw02739001002_02101_00001_nrcalong_cal.jpg", "pane"), TOKEN_SAVE_ID]) {
      expect(isValidRequestId(id)).toBe(true);
    }
    expect(isValidRequestId("g1:thumb:a b")).toBe(false);
    expect(isValidRequestId("")).toBe(false);
    expect(isValidRequestId("x".repeat(513))).toBe(false);
  });
});
