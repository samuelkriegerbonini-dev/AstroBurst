import { useEffect, useRef, useCallback, useState } from "react";
import { renderStfInWorker, cancelPendingRenders, setWorkerPixels, clearWorkerPixels } from "../../utils/stfworker";
import { getGpuSingleton, getGpuState, onGpuLost, type GpuResources as GpuSingleton } from "../../infrastructure/gpu/GpuSingleton";

interface GpuResources {
  uniformBuffer: GPUBuffer;
  texture: GPUTexture;
  bindGroup: GPUBindGroup;
}

interface GpuRendererProps {
  rawData: Float32Array | null;
  width: number;
  height: number;
  dataMin: number;
  dataMax: number;
  shadow?: number;
  midtone?: number;
  highlight?: number;
  className?: string;
}

export default function GpuRenderer({
  rawData,
  width,
  height,
  dataMin,
  dataMax,
  shadow = 0,
  midtone = 0.5,
  highlight = 1,
  className = "",
}: GpuRendererProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const fallbackRef = useRef(false);
  const resourcesRef = useRef<GpuResources | null>(null);
  const prevDimsRef = useRef({ w: 0, h: 0 });
  const uploadedDataRef = useRef<Float32Array | null>(null);
  const [gpuReady, setGpuReady] = useState(false);
  const [gpuGen, setGpuGen] = useState(0);
  const renderSeqRef = useRef(0);
  const rafRef = useRef<number | null>(null);
  const contextConfiguredRef = useRef(false);
  const uniformScratchRef = useRef<Float32Array | null>(null);
  const lastUniformWriteRef = useRef<Float32Array | null>(null);

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
    resourcesRef.current = null;
    uploadedDataRef.current = null;
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
        size: 32,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
      });

      const texture = device.createTexture({
        size: [w, h, 1],
        format: "r32float",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });

      const bindGroup = device.createBindGroup({
        layout: pipeline.getBindGroupLayout(0),
        entries: [
          { binding: 0, resource: { buffer: uniformBuffer } },
          { binding: 1, resource: texture.createView() },
        ],
      });

      resourcesRef.current = { uniformBuffer, texture, bindGroup };
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

    let uniforms = uniformScratchRef.current;
    if (!uniforms) {
      uniforms = new Float32Array(8);
      uniformScratchRef.current = uniforms;
    }
    uniforms[0] = dataMin; uniforms[1] = dataMax; uniforms[2] = shadow; uniforms[3] = midtone;
    uniforms[4] = highlight; uniforms[5] = w; uniforms[6] = h; uniforms[7] = 0;

    const last = lastUniformWriteRef.current;
    let unchanged = last !== null;
    if (last) {
      for (let i = 0; i < 8; i++) {
        if (last[i] !== uniforms[i]) { unchanged = false; break; }
      }
    }
    if (!unchanged) {
      device.queue.writeBuffer(res.uniformBuffer, 0, uniforms as Float32Array<ArrayBuffer>);
      if (!lastUniformWriteRef.current) lastUniformWriteRef.current = new Float32Array(8);
      lastUniformWriteRef.current.set(uniforms);
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
  }, [rawData, width, height, dataMin, dataMax, shadow, midtone, highlight, destroyGPUResources]);

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
        dataMin,
        dataMax,
        shadow,
        midtone,
        highlight,
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
  }, [rawData, width, height, dataMin, dataMax, shadow, midtone, highlight]);
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
