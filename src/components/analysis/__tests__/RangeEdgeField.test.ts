import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import RangeEdgeField, { type RangeEdgeFieldProps } from "../RangeEdgeField";
import DecimalField from "../../ui/DecimalField";
import type { PlotMapping } from "../../../utils/spectrumRange";

type FieldProps = Record<string, unknown> & {
  value: string;
  onChange: (text: string) => void;
  onBlur: () => void;
  onKeyDown: (e: { key: string }) => void;
};

const xs = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
const m: PlotMapping = { width: 400, padLeft: 62, padRight: 12, xMin: xs[0], xMax: xs[39], xValues: xs, n: 40 };

const onCommit = vi.fn<(channel: number) => void>();

function propsOf(extra: Partial<RangeEdgeFieldProps>): RangeEdgeFieldProps {
  return {
    edge: "range:z0",
    channel: 5,
    channelCount: 40,
    mapping: null,
    unit: "μm",
    channelLabel: "Range first channel z0",
    axisLabel: "Range first channel, in μm",
    placeholder: "z0",
    onCommit,
    ...extra,
  };
}

function field(props: RangeEdgeFieldProps): FieldProps {
  const tree = hooks.render(RangeEdgeField, props) as ReactNode;
  expect(isValidElement(tree) && tree.type === DecimalField).toBe(true);
  return (tree as { props: FieldProps }).props;
}

beforeEach(() => {
  hooks.mount();
  onCommit.mockReset();
});

describe("RangeEdgeField", () => {
  it("is the channel box of today without a mapping", () => {
    const props = propsOf({});
    const box = field(props);
    expect(box.integer).toBe(true);
    expect(box.min).toBe(0);
    expect(box.max).toBe(39);
    expect(box.value).toBe("5");
    expect(box["aria-label"]).toBe("Range first channel z0");
    expect(box["data-range-edge"]).toBe("range:z0");
    expect(box.placeholder).toBe("z0");
    field(props).onChange("7");
    field(props).onBlur();
    expect(onCommit).toHaveBeenCalledWith(7);
    onCommit.mockReset();
    field(props).onChange("40");
    field(props).onBlur();
    expect(onCommit).not.toHaveBeenCalled();
    field(props).onChange("9");
    field(props).onKeyDown({ key: "Escape" });
    expect(field(props).value).toBe("5");
    hooks.mount();
    const empty = field(propsOf({ channel: null }));
    expect(empty.value).toBe("");
    expect(empty.optional).toBe(true);
  });

  it("shows and accepts axis values with a mapping", () => {
    const props = propsOf({ channel: 20, mapping: m });
    const box = field(props);
    expect(box.value).toBe("0.6575");
    expect(box["aria-label"]).toBe("Range first channel, in μm");
    expect(box.title).toBe("channel 20");
    expect(box.integer).toBeUndefined();
    expect(box.min).toBeUndefined();
    expect(box.max).toBeUndefined();
    expect(box["data-range-edge"]).toBe("range:z0");
    field(props).onChange("0.6580");
    field(props).onKeyDown({ key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(25);
    onCommit.mockReset();
    field(props).onChange("0.65753");
    field(props).onBlur();
    expect(onCommit).not.toHaveBeenCalled();
    expect(field(props).value).toBe("0.6575");
    field(props).onChange("0.7");
    expect(field(props).error).toBe("Use a value from 0.6555 to 0.6594 μm");
    field(props).onBlur();
    expect(onCommit).not.toHaveBeenCalled();
    expect(field(props).value).toBe("0.7");
  });

  it("follows a channel set from outside", () => {
    const props = propsOf({ channel: 20, mapping: m });
    field(props).onChange("0.6560");
    expect(field(props).value).toBe("0.6560");
    const moved = field({ ...props, channel: 30 });
    expect(moved.value).toBe("0.6585");
    expect(moved.title).toBe("channel 30");
    expect(moved.error ?? null).toBeNull();
    expect(field({ ...props, channel: 30 }).value).toBe("0.6585");
  });
});
