import { beforeEach, describe, expect, it, vi } from "vitest";

const getHeader = vi.hoisted(() => vi.fn((_path: string): Promise<Record<string, string>> => Promise.resolve({})));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../services/header", () => ({ getHeader }));

import { useImageUnit } from "../useImageUnit";
import { hooks } from "../../test/hookRuntime";

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}

function render(path: string | null, enabled: boolean): string | null {
  return hooks.render(({ p, e }: { p: string | null; e: boolean }) => useImageUnit(p, e), { p: path, e: enabled }) as string | null;
}

beforeEach(() => {
  hooks.mount();
  getHeader.mockReset();
});

describe("useImageUnit", () => {
  it("reads BUNIT once per path and gives null until it arrives", async () => {
    let resolve: (header: Record<string, string>) => void = () => {};
    getHeader.mockImplementation(() => new Promise((r) => (resolve = r)));
    expect(render("a.fits", true)).toBeNull();
    resolve({ BUNIT: "'MJy/sr'" });
    await flush();
    expect(render("a.fits", true)).toBe("MJy/sr");
    expect(render("a.fits", true)).toBe("MJy/sr");
    expect(getHeader).toHaveBeenCalledTimes(1);
    expect(getHeader).toHaveBeenCalledWith("a.fits");
  });

  it("never asks for the header without a path or while disabled", async () => {
    getHeader.mockResolvedValue({ BUNIT: "MJy/sr" });
    expect(render("a.fits", false)).toBeNull();
    await flush();
    expect(render("a.fits", false)).toBeNull();
    hooks.mount();
    expect(render(null, true)).toBeNull();
    await flush();
    expect(render(null, true)).toBeNull();
    expect(getHeader).not.toHaveBeenCalled();
  });

  it("gives null on a rejection and for a header without BUNIT", async () => {
    getHeader.mockRejectedValue(new Error("not a FITS file"));
    render("bad.fits", true);
    await flush();
    expect(render("bad.fits", true)).toBeNull();
    hooks.mount();
    getHeader.mockResolvedValue({ TELESCOP: "Mayall" });
    render("b.fits", true);
    await flush();
    expect(render("b.fits", true)).toBeNull();
  });

  it("does not show the unit of the previous path", async () => {
    getHeader.mockResolvedValueOnce({ BUNIT: "MJy/sr" }).mockReturnValueOnce(new Promise(() => {}));
    render("a.fits", true);
    await flush();
    expect(render("a.fits", true)).toBe("MJy/sr");
    expect(render("b.fits", true)).toBeNull();
    await flush();
    expect(render("b.fits", true)).toBeNull();
  });
});
