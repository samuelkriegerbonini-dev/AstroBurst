import { useEffect, useRef, useCallback, useState } from "react";
import { renderStfInWorker, cancelPendingRenders, setWorkerPixels, clearWorkerPixels } from "../../utils/stfworker";
import {
  STF_UNIFORM_WORD,
  getGpuSingleton,
  getGpuState,
  onGpuLost,
  type GpuResources as GpuSingleton,
} from "../../infrastructure/gpu/GpuSingleton";
import { LUT_BYTES, lutNodataRgb, type DisplayTransfer } from "../../utils/displayTransfer";

interface GpuResources {
  uniformBuffer: GPUBuffer;
  texture: GPUTexture;
  lutTexture: GPUTexture;
  bindGroup: GPUBindGroup;
}

interface GpuRendererProps {
  rawData: Float32Array | null;
  width: number;
  height: number;
  transfer: DisplayTransfer;
  lut: Uint8Array;
  className?: string;
}

const UNIFORM_BYTES = 64;
const UNIFORM_WORDS = 16;

interface UniformScratch {
  buffer: ArrayBuffer;
  f32: Float32Array;
  u32: Uint32Array;
}

function makeScratch(): UniformScratch {
  const buffer = new ArrayBuffer(UNIFORM_BYTES);
  return { buffer, f32: new Float32Array(buffer), u32: new Uint32Array(buffer) };
}

function fillUniforms(
  s: UniformScratch,
  t: DisplayTransfer,
  w: number,
  h: number,
  nodata: readonly [number, number, number],
): void {
  const { f32, u32 } = s;
  const word = STF_UNIFORM_WORD;
  f32[word.vmin] = t.vmin;
  f32[word.vmax] = t.vmax;
  f32[word.shadow] = t.shadow;
  f32[word.midtone] = t.midtone;
  f32[word.highlight] = t.highlight;
  f32[word.asinh_a] = t.asinhA;
  f32[word.power] = t.power;
  f32[word.tex_w] = w;
  f32[word.tex_h] = h;
  f32[word.nodata_r] = nodata[0] / 255;
  f32[word.nodata_g] = nodata[1] / 255;
  f32[word.nodata_b] = nodata[2] / 255;
  u32[word.stretch_kind] = t.stretchKind;
  u32[word.invert] = t.invert ? 1 : 0;
  u32[14] = 0;
  u32[15] = 0;
}

export default function GpuRenderer({
  rawData,
  width,
  height,
  transfer,
  lut,
  className = "",
}: GpuRendererProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fallbackRef = useRef(false);
  const resourcesRef = useRef<GpuResources | null>(null);
  const prevDimsRef = useRef({ w: 0, h: 0 });
  const uploadedDataRef = useRef<Float32Array | null>(null);
  const uploadedLutRef = useRef<Uint8Array | null>(null);
  const [gpuReady, setGpuReady] = useState(false);
  const [gpuGen, setGpuGen] = useState(0);
  const renderSeqRef = useRef(0);
  const rafRef = useRef<number | null>(null);
  const contextConfiguredRef = useRef(false);
  const uniformScratchRef = useRef<UniformScratch | null>(null);
  const lastUniformWriteRef = useRef<Uint32Array | null>(null);

  useEffect(() => {
    let cancelled = false;
    getGpuSingleton().then((gpu: GpuSingleton | null) => {
      if (cancelled) return;
      if (!gpu) fallbackRef.current = true;
      setGpuReady(true);
    });
    return () => { cancelled = true; };
  }, []);

  const destroyGPUResources = useCallback(() => {
    const res = resourcesRef.current;
    if (!res) return;
    if (res.uniformBuffer) res.uniformBuffer.destroy();
    if (res.texture) res.texture.destroy();
    if (res.lutTexture) res.lutTexture.destroy();
    resourcesRef.current = null;
    uploadedDataRef.current = null;
    uploadedLutRef.current = null;
    contextConfiguredRef.current = false;
    lastUniformWriteRef.current = null;
  }, []);

  useEffect(() => {
    return () => {
      destroyGPUResources();
      cancelPendingRenders();
      clearWorkerPixels();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [destroyGPUResources]);

  useEffect(() => {
    const unsubscribe = onGpuLost(() => {
      if (fallbackRef.current) return;
      fallbackRef.current = true;
      destroyGPUResources();
      setGpuGen((g) => g + 1);
    });
    return unsubscribe;
  }, [destroyGPUResources]);

  const workerPixelsReadyRef = useRef(false);

  useEffect(() => {
    if (!rawData || !width || !height || !gpuReady || !fallbackRef.current) {
      workerPixelsReadyRef.current = false;
      return;
    }
    setWorkerPixels(rawData, width, height);
    workerPixelsReadyRef.current = true;
  }, [rawData, width, height, gpuReady, gpuGen]);

  const renderGPU = useCallback(() => {
    const gpu = getGpuState();
    if (!gpu || !rawData || !canvasRef.current) return;
    const { device, pipeline, format } = gpu;
    const w = width;
    const h = height;

    const maxTex = device.limits.maxTextureDimension2D;
    if (w > maxTex || h > maxTex) {
      fallbackRef.current = true;
      setGpuGen((g) => g + 1);
      return;
    }

    const canvas = canvasRef.current;
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      contextConfiguredRef.current = false;
    }

    const context = canvas.getContext("webgpu") as GPUCanvasContext;
    if (!contextConfiguredRef.current) {
      context.configure({ device, format, alphaMode: "premultiplied" });
      contextConfiguredRef.current = true;
    }

    const dimsChanged = prevDimsRef.current.w !== w || prevDimsRef.current.h !== h;

    if (!resourcesRef.current || dimsChanged) {
      destroyGPUResources();

      const uniformBuffer = device.createBuffer({
        size: UNIFORM_BYTES,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
      });

      const texture = device.createTexture({
        size: [w, h, 1],
        format: "r32float",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });

      const lutTexture = device.createTexture({
        size: [256, 1, 1],
        format: "rgba8unorm",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });

      const bindGroup = device.createBindGroup({
        layout: pipeline.getBindGroupLayout(0),
        entries: [
          { binding: 0, resource: { buffer: uniformBuffer } },
          { binding: 1, resource: texture.createView() },
          { binding: 2, resource: lutTexture.createView() },
        ],
      });

      resourcesRef.current = { uniformBuffer, texture, lutTexture, bindGroup };
      prevDimsRef.current = { w, h };
      lastUniformWriteRef.current = null;
    }

    const res = resourcesRef.current;

    if (uploadedDataRef.current !== rawData) {
      device.queue.writeTexture(
        { texture: res.texture },
        rawData as Float32Array<ArrayBuffer>,
        { bytesPerRow: w * 4 },
        [w, h, 1]
      );
      uploadedDataRef.current = rawData;
    }

    if (uploadedLutRef.current !== lut && lut.length >= LUT_BYTES) {
      device.queue.writeTexture(
        { texture: res.lutTexture },
        lut as Uint8Array<ArrayBuffer>,
        { bytesPerRow: LUT_BYTES, rowsPerImage: 1 },
        [256, 1, 1]
      );
      uploadedLutRef.current = lut;
    }

    let scratch = uniformScratchRef.current;
    if (!scratch) {
      scratch = makeScratch();
      uniformScratchRef.current = scratch;
    }
    fillUniforms(scratch, transfer, w, h, lutNodataRgb(lut));

    const last = lastUniformWriteRef.current;
    let unchanged = last !== null;
    if (last) {
      for (let i = 0; i < UNIFORM_WORDS; i++) {
        if (last[i] !== scratch.u32[i]) { unchanged = false; break; }
      }
    }
    if (!unchanged) {
      device.queue.writeBuffer(res.uniformBuffer, 0, scratch.buffer);
      if (!lastUniformWriteRef.current) lastUniformWriteRef.current = new Uint32Array(UNIFORM_WORDS);
      lastUniformWriteRef.current.set(scratch.u32);
    }

    const commandEncoder = device.createCommandEncoder();
    const renderPassDescriptor: GPURenderPassDescriptor = {
      colorAttachments: [{
        view: context.getCurrentTexture().createView(),
        clearValue: { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
        loadOp: "clear",
        storeOp: "store",
      }],
    };

    const passEncoder = commandEncoder.beginRenderPass(renderPassDescriptor);
    passEncoder.setPipeline(pipeline);
    passEncoder.setBindGroup(0, res.bindGroup);
    passEncoder.draw(6);
    passEncoder.end();

    device.queue.submit([commandEncoder.finish()]);
  }, [rawData, width, height, transfer, lut, destroyGPUResources]);

  const cpuBusyRef = useRef(false);
  const cpuPendingRef = useRef(false);
  const renderCPUWorkerRef = useRef<() => void>(() => {});

  const renderCPUWorker = useCallback(async () => {
    if (!rawData || !width || !height) return;
    if (cpuBusyRef.current) {
      cpuPendingRef.current = true;
      return;
    }
    cpuBusyRef.current = true;
    const seq = ++renderSeqRef.current;

    try {
      const sendPixels = !workerPixelsReadyRef.current;
      const result = await renderStfInWorker({
        pixels: sendPixels ? rawData : undefined,
        width: sendPixels ? width : undefined,
        height: sendPixels ? height : undefined,
        transfer,
        lut,
      });

      if (renderSeqRef.current !== seq) return;

      const canvas = canvasRef.current;
      if (!canvas || !result.bitmap) return;
      const w = result.width;
      const h = result.height;

      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }

      const ctx = canvas.getContext("bitmaprenderer");
      if (ctx) {
        ctx.transferFromImageBitmap(result.bitmap);
      } else {
        const ctx2d = canvas.getContext("2d")!;
        ctx2d.drawImage(result.bitmap, 0, 0);
        result.bitmap.close();
      }
    } finally {
      cpuBusyRef.current = false;
      if (cpuPendingRef.current) {
        cpuPendingRef.current = false;
        renderCPUWorkerRef.current();
      }
    }
  }, [rawData, width, height, transfer, lut]);
  renderCPUWorkerRef.current = renderCPUWorker;

  useEffect(() => {
    if (!gpuReady || !rawData) return;

    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      if (fallbackRef.current) {
        renderCPUWorker();
      } else {
        renderGPU();
      }
    });
  }, [gpuReady, rawData, renderCPUWorker, renderGPU, gpuGen]);

  if (!gpuReady) {
    return <div className={`animate-pulse bg-zinc-800/50 w-full h-full ${className}`} />;
  }

  return (
    <canvas
      key={fallbackRef.current ? "cpu-canvas" : "gpu-canvas"}
      ref={canvasRef}
      className={className}
      style={{ display: "block" }}
    />
  );
}
