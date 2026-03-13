import { useEffect, useRef, useCallback, useState, useMemo } from "react";
import { renderStfInWorker, cancelPendingRenders, setWorkerPixels } from "../../utils/stfWorker.ts";
import { getGpuSingleton, getGpuState } from "../../context/Gpucontext";

const MAX_DISPLAY_DIM = 4096;

function clampDimensions(w, h, maxDim) {
  if (w <= maxDim && h <= maxDim) return { width: w, height: h, scale: 1 };
  const scale = maxDim / Math.max(w, h);
  return {
    width: Math.round(w * scale),
    height: Math.round(h * scale),
    scale,
  };
}

function downsampleF32(src, srcW, srcH, dstW, dstH) {
  const dst = new Float32Array(dstW * dstH);
  const xRatio = srcW / dstW;
  const yRatio = srcH / dstH;
  for (let y = 0; y < dstH; y++) {
    const srcY = Math.min(Math.floor(y * yRatio), srcH - 1);
    const srcRowOff = srcY * srcW;
    const dstRowOff = y * dstW;
    for (let x = 0; x < dstW; x++) {
      const srcX = Math.min(Math.floor(x * xRatio), srcW - 1);
      dst[dstRowOff + x] = src[srcRowOff + srcX];
    }
  }
  return dst;
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
                                    }) {
  const canvasRef = useRef(null);
  const fallbackRef = useRef(false);
  const resourcesRef = useRef(null);
  const prevDimsRef = useRef({ w: 0, h: 0 });
  const uploadedDataRef = useRef(null);
  const [gpuReady, setGpuReady] = useState(false);
  const renderSeqRef = useRef(0);
  const rafRef = useRef(null);
  const contextConfiguredRef = useRef(false);

  const display = useMemo(
    () => clampDimensions(width, height, MAX_DISPLAY_DIM),
    [width, height],
  );

  const displayData = useMemo(() => {
    if (!rawData || !width || !height) return null;
    if (display.scale === 1) return rawData;
    return downsampleF32(rawData, width, height, display.width, display.height);
  }, [rawData, width, height, display]);

  useEffect(() => {
    let cancelled = false;
    getGpuSingleton().then((gpu) => {
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
  }, []);

  useEffect(() => {
    return () => {
      destroyGPUResources();
      cancelPendingRenders();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [destroyGPUResources]);

  const workerPixelsReadyRef = useRef(false);
  const pixelsGenRef = useRef(0);

  useEffect(() => {
    if (!rawData || !width || !height) {
      workerPixelsReadyRef.current = false;
      return;
    }
    workerPixelsReadyRef.current = false;
    const gen = ++pixelsGenRef.current;
    setWorkerPixels(rawData, width, height).then(() => {
      if (pixelsGenRef.current === gen) {
        workerPixelsReadyRef.current = true;
      }
    });
  }, [rawData, width, height]);

  const renderGPU = useCallback(() => {
    const gpu = getGpuState();
    if (!gpu || !displayData || !canvasRef.current) return;
    const { device, pipeline, format } = gpu;
    const w = display.width;
    const h = display.height;

    const canvas = canvasRef.current;
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      contextConfiguredRef.current = false;
    }

    const context = canvas.getContext("webgpu");
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
    }

    const res = resourcesRef.current;

    if (uploadedDataRef.current !== displayData) {
      device.queue.writeTexture(
        { texture: res.texture },
        displayData,
        { bytesPerRow: w * 4 },
        [w, h, 1]
      );
      uploadedDataRef.current = displayData;
    }

    const uniforms = new Float32Array([dataMin, dataMax, shadow, midtone, highlight, w, h, 0]);
    device.queue.writeBuffer(res.uniformBuffer, 0, uniforms);

    const commandEncoder = device.createCommandEncoder();
    const renderPassDescriptor = {
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
  }, [displayData, display, dataMin, dataMax, shadow, midtone, highlight, destroyGPUResources]);

  const renderCPUWorker = useCallback(async () => {
    if (!rawData || !width || !height) return;
    const seq = ++renderSeqRef.current;

    const needsDownsample = display.scale < 1;
    const sendPixels = !workerPixelsReadyRef.current;
    const result = await renderStfInWorker({
      pixels: sendPixels ? rawData : undefined,
      width: sendPixels ? width : undefined,
      height: sendPixels ? height : undefined,
      dstWidth: needsDownsample ? display.width : undefined,
      dstHeight: needsDownsample ? display.height : undefined,
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
      const ctx2d = canvas.getContext("2d");
      ctx2d.drawImage(result.bitmap, 0, 0);
      result.bitmap.close();
    }
  }, [rawData, width, height, display, dataMin, dataMax, shadow, midtone, highlight]);

  useEffect(() => {
    if (!gpuReady || (!displayData && !rawData)) return;

    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      if (fallbackRef.current) {
        renderCPUWorker();
      } else {
        renderGPU();
      }
    });
  }, [gpuReady, displayData, rawData, renderCPUWorker, renderGPU]);

  if (!gpuReady) {
    return <div className={`animate-pulse bg-zinc-800/50 ${className}`} style={{ aspectRatio: width/height }} />;
  }

  return (
    <canvas
      key={fallbackRef.current ? "cpu-canvas" : "gpu-canvas"}
      ref={canvasRef}
      className={`max-w-full h-auto ${className}`}
      style={{ imageRendering: display.scale < 1 ? "auto" : "pixelated" }}
    />
  );
}
