import { useEffect, useRef, useCallback, useState } from "react";
import { getGpuSingleton, getGpuState, onGpuLost } from "../../infrastructure/gpu/GpuSingleton";
import type { RawRgbPixelData, StfParams } from "../../shared/types";

interface GpuRgbResources {
  uniformBuffer: GPUBuffer;
  texR: GPUTexture;
  texG: GPUTexture;
  texB: GPUTexture;
  bindGroup: GPUBindGroup;
}

interface GpuRgbRendererProps {
  rgb: RawRgbPixelData;
  stfR: StfParams;
  stfG: StfParams;
  stfB: StfParams;
  className?: string;
}

export default function GpuRgbRenderer({ rgb, stfR, stfG, stfB, className = "" }: GpuRgbRendererProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const resourcesRef = useRef<GpuRgbResources | null>(null);
  const prevDimsRef = useRef({ w: 0, h: 0 });
  const uploadedRgbRef = useRef<RawRgbPixelData | null>(null);
  const [gpuReady, setGpuReady] = useState(false);
  const [gpuOk, setGpuOk] = useState(true);
  const rafRef = useRef<number | null>(null);
  const contextConfiguredRef = useRef(false);
  const uniformScratchRef = useRef<Float32Array | null>(null);
  const lastUniformWriteRef = useRef<Float32Array | null>(null);

  useEffect(() => {
    let cancelled = false;
    getGpuSingleton().then((gpu) => {
      if (cancelled) return;
      if (!gpu) setGpuOk(false);
      setGpuReady(true);
    });
    return () => { cancelled = true; };
  }, []);

  const destroyGPUResources = useCallback(() => {
    const res = resourcesRef.current;
    if (!res) return;
    res.uniformBuffer.destroy();
    res.texR.destroy();
    res.texG.destroy();
    res.texB.destroy();
    resourcesRef.current = null;
    uploadedRgbRef.current = null;
    contextConfiguredRef.current = false;
    lastUniformWriteRef.current = null;
  }, []);

  useEffect(() => {
    return () => {
      destroyGPUResources();
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [destroyGPUResources]);

  useEffect(() => {
    const unsubscribe = onGpuLost(() => {
      destroyGPUResources();
      setGpuOk(false);
    });
    return unsubscribe;
  }, [destroyGPUResources]);

  const renderGPU = useCallback(() => {
    const gpu = getGpuState();
    if (!gpu || !canvasRef.current) return;
    const { device, rgbPipeline, format } = gpu;
    const w = rgb.width;
    const h = rgb.height;

    const maxTex = device.limits.maxTextureDimension2D;
    if (w > maxTex || h > maxTex) {
      setGpuOk(false);
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
        size: 80,
        usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
      });

      const makeTexture = () =>
        device.createTexture({
          size: [w, h, 1],
          format: "r32float",
          usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
        });
      const texR = makeTexture();
      const texG = makeTexture();
      const texB = makeTexture();

      const bindGroup = device.createBindGroup({
        layout: rgbPipeline.getBindGroupLayout(0),
        entries: [
          { binding: 0, resource: { buffer: uniformBuffer } },
          { binding: 1, resource: texR.createView() },
          { binding: 2, resource: texG.createView() },
          { binding: 3, resource: texB.createView() },
        ],
      });

      resourcesRef.current = { uniformBuffer, texR, texG, texB, bindGroup };
      prevDimsRef.current = { w, h };
      uploadedRgbRef.current = null;
      lastUniformWriteRef.current = null;
    }

    const res = resourcesRef.current;

    if (uploadedRgbRef.current !== rgb) {
      const upload = (tex: GPUTexture, data: Float32Array) =>
        device.queue.writeTexture(
          { texture: tex },
          data as Float32Array<ArrayBuffer>,
          { bytesPerRow: w * 4 },
          [w, h, 1],
        );
      upload(res.texR, rgb.r.data);
      upload(res.texG, rgb.g.data);
      upload(res.texB, rgb.b.data);
      uploadedRgbRef.current = rgb;
    }

    let u = uniformScratchRef.current;
    if (!u) {
      u = new Float32Array(20);
      uniformScratchRef.current = u;
    }
    u[0] = rgb.r.min; u[1] = rgb.r.max; u[2] = stfR.shadow; u[3] = stfR.midtone;
    u[4] = rgb.g.min; u[5] = rgb.g.max; u[6] = stfG.shadow; u[7] = stfG.midtone;
    u[8] = rgb.b.min; u[9] = rgb.b.max; u[10] = stfB.shadow; u[11] = stfB.midtone;
    u[12] = stfR.highlight; u[13] = stfG.highlight; u[14] = stfB.highlight; u[15] = 0;
    u[16] = w; u[17] = h; u[18] = 0; u[19] = 0;

    const last = lastUniformWriteRef.current;
    let unchanged = last !== null;
    if (last) {
      for (let i = 0; i < 20; i++) {
        if (last[i] !== u[i]) { unchanged = false; break; }
      }
    }
    if (!unchanged) {
      device.queue.writeBuffer(res.uniformBuffer, 0, u as Float32Array<ArrayBuffer>);
      if (!lastUniformWriteRef.current) lastUniformWriteRef.current = new Float32Array(20);
      lastUniformWriteRef.current.set(u);
    }

    const commandEncoder = device.createCommandEncoder();
    const passEncoder = commandEncoder.beginRenderPass({
      colorAttachments: [{
        view: context.getCurrentTexture().createView(),
        clearValue: { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
        loadOp: "clear",
        storeOp: "store",
      }],
    });
    passEncoder.setPipeline(rgbPipeline);
    passEncoder.setBindGroup(0, res.bindGroup);
    passEncoder.draw(6);
    passEncoder.end();

    device.queue.submit([commandEncoder.finish()]);
  }, [rgb, stfR, stfG, stfB, destroyGPUResources]);

  useEffect(() => {
    if (!gpuReady || !gpuOk) return;
    if (rafRef.current) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      renderGPU();
    });
  }, [gpuReady, gpuOk, renderGPU]);

  if (!gpuReady) {
    return <div className={`animate-pulse bg-zinc-800/50 w-full h-full ${className}`} />;
  }

  if (!gpuOk) {
    return null;
  }

  return (
    <canvas
      ref={canvasRef}
      className={className}
      style={{ display: "block" }}
    />
  );
}
