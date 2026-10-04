import type { RawPixelData } from "../shared/types";

export type PixelHolder<T extends object> = T & { readonly data: Float32Array };

export interface RawPixelsResultLike {
  width: number;
  height: number;
  dataMin: number;
  dataMax: number;
  pixels: Float32Array;
}

export function makePixelHolder<T extends object>(meta: T, data: Float32Array): PixelHolder<T> {
  const holder = { ...meta };
  Object.defineProperty(holder, "data", { value: data, enumerable: false, writable: true, configurable: true });
  return holder as PixelHolder<T>;
}

export function rawPixelsHolder(result: RawPixelsResultLike): RawPixelData {
  return makePixelHolder(
    { width: result.width, height: result.height, min: result.dataMin, max: result.dataMax },
    result.pixels,
  );
}
