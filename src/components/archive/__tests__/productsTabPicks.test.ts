import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import type { MastMission, MastProduct } from "../../../shared/types/mast";

vi.mock("react", async (original) => {
  const { reactWithFakeHooks } = await import("../../../test/hookRuntime");
  return reactWithFakeHooks(await original<Record<string, unknown>>());
});

import { hooks } from "../../../test/hookRuntime";
import { elementsOf } from "../../../test/elementTree";
import ProductsTab from "../ProductsTab";
import { DownloadBarView } from "../DownloadBar";
import { archiveReducer, createInitialState, type ArchiveAction, type ArchiveState } from "../state/archiveReducer";
import type { ArchiveController } from "../state/archiveController";
import { MISSIONS, product, productList, row } from "./fixtures";

const ID = "jw02739-o001_t001_nircam";
const KEY = `jwst:${ID}`;
const JWST = MISSIONS.find((m) => m.id === "jwst") as MastMission;

function stubController(): ArchiveController {
  const fns: Record<string | symbol, unknown> = {};
  return new Proxy(fns, { get: (target, prop) => (target[prop] ??= vi.fn()) }) as unknown as ArchiveController;
}

function listed(mode: "opens" | "everything", products: MastProduct[] = productList().products): ArchiveState {
  const actions: ArchiveAction[] = [
    { type: "missionsLoaded", missions: MISSIONS },
    { type: "productsStarted", key: KEY, requestId: "g1:products", at: 0 },
    { type: "productsLoaded", key: KEY, requestId: "g1:products", list: { ...productList(), products } },
    { type: "productModeSet", mode },
  ];
  return actions.reduce(archiveReducer, createInitialState());
}

function render(state: ArchiveState): unknown {
  return hooks.render(ProductsTab as (props: Parameters<typeof ProductsTab>[0]) => unknown, { state, ctl: stubController(), row: row(ID), mission: JWST });
}

function bar(tree: unknown) {
  const el = elementsOf(tree).find((e) => e.type === DownloadBarView) as ReactElement<{ count: number; bytes: number }> | undefined;
  if (!el) throw new Error("no download bar");
  return el.props;
}

function pickOf(tree: unknown): (p: MastProduct) => void {
  const section = elementsOf<{ onPick?: ((p: MastProduct) => void) | null }>(tree).find((e) => typeof e.props.onPick === "function");
  if (!section?.props.onPick) throw new Error("no pickable section");
  return section.props.onPick;
}

beforeEach(() => hooks.mount());

describe("files picked in a section that is no longer shown", () => {
  it("are neither counted nor sent once the section collapses", () => {
    const aux = productList().products.find((p) => p.cls === "auxiliary") as MastProduct;
    pickOf(render(listed("everything")))(aux);
    expect(bar(render(listed("everything"))).count).toBe(1);
    expect(bar(render(listed("opens"))).count).toBe(0);
  });
});

describe("Select science files", () => {
  it("picks the science files the app opens, not the intermediate stages or the auxiliary files", () => {
    const products = [
      product("a_i2d.fits", "image", 5_000, { suffix: "i2d" }),
      product("a_cal.fits", "image", 3_000, { suffix: "cal" }),
      product("a_rate.fits", "image", 2_000, { suffix: "rate" }),
      product("a_uncal.fits", "ramp", 9_000, { suffix: "uncal" }),
      product("x_jit.fits", "auxiliary", 5, { suffix: "jit" }),
    ];
    const tree = render(listed("everything", products));
    const button = elementsOf<{ "data-testid"?: string; onClick?: () => void; children?: unknown }>(tree).find((e) => e.props["data-testid"] === "mast-select-science");
    expect(button?.props.children).toBe("Select science files (2)");
    button?.props.onClick?.();
    expect(bar(render(listed("everything", products)))).toMatchObject({ count: 2, bytes: 8_000 });
  });
});
