import { useEffect, useRef, useCallback, useState } from "react";
import { STF_SHADER } from "../utils/shaders";

/**
 * GpuRenderer — renders FITS f32 data via WebGPU compute shader.
 * Falls back to Canvas 2D when WebGPU is unavailable.
 *
 * Props:
 *   rawData   - Float32Array of pixel values (row-major)
 *   width     - image width in pixels
 *   height    - image height in pixels
 *   dataMin   - minimum data value
 *   dataMax   - maximum data value
 *   shadow    - STF shadow (black point) [0..1]
 *   midtone   - STF midtone balance     (0..1)
 *   highlight - STF highlight (white point) [0..1]
 *   className - optional CSS class
 */
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
  const gpuRef = useRef(null);        
  const fallbackRef = useRef(false);
  const [gpuReady, setGpuReady] = useState(false);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      if (!navigator.gpu) {
        fallbackRef.current = true;
        setGpuReady(true);
        return;
      }
      try {
        const adapter = await navigator.gpu.requestAdapter();
        if (!adapter || cancelled) { fallbackRef.current = true; setGpuReady(true); return; }
        const device = await adapter.requestDevice();
        if (cancelled) return;

        const module = device.createShaderModule({ code: STF_SHADER });

        const pipeline = device.createComputePipeline({
          layout: "auto",
          compute: { module, entryPoint: "main" },
        });

        gpuRef.current = { device, pipeline };
        setGpuReady(true);
      } catch (e) {
        console.warn("WebGPU init failed, falling back to Canvas2D:", e);
        fallbackRef.current = true;
        setGpuReady(true);
      }
    })();

    return () => { cancelled = true; };
  }, []);

  
  useEffect(() => {
    if (!gpuReady || !rawData || !width || !height) return;
    if (fallbackRef.current) {
      renderCPU();
    } else {
      renderGPU();
    }
  }, [gpuReady, rawData, width, height, dataMin, dataMax, shadow, midtone, highlight]);

  
  const renderGPU = useCallback(async () => {
    const gpu = gpuRef.current;
    if (!gpu) return;
    const { device, pipeline } = gpu;

    const totalPixels = width * height;

    
    const uniformData = new ArrayBuffer(32);
    const u32View = new Uint32Array(uniformData);
    const f32View = new Float32Array(uniformData);
    u32View[0] = width;
    u32View[1] = height;
    f32View[2] = dataMin;
    f32View[3] = dataMax;
    f32View[4] = shadow;
    f32View[5] = midtone;
    f32View[6] = highlight;
    f32View[7] = 0; 

    const uniformBuffer = device.createBuffer({
      size: 32,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    device.queue.writeBuffer(uniformBuffer, 0, uniformData);

    
    const pixelBuffer = device.createBuffer({
      size: totalPixels * 4,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    device.queue.writeBuffer(pixelBuffer, 0, rawData);

    
    const outputBuffer = device.createBuffer({
      size: totalPixels * 4,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_SRC,
    });

    
    const stagingBuffer = device.createBuffer({
      size: totalPixels * 4,
      usage: GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST,
    });

    const bindGroup = device.createBindGroup({
      layout: pipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: uniformBuffer } },
        { binding: 1, resource: { buffer: pixelBuffer } },
        { binding: 2, resource: { buffer: outputBuffer } },
      ],
    });

    const encoder = device.createCommandEncoder();
    const pass = encoder.beginComputePass();
    pass.setPipeline(pipeline);
    pass.setBindGroup(0, bindGroup);
    pass.dispatchWorkgroups(Math.ceil(totalPixels / 256));
    pass.end();

    encoder.copyBufferToBuffer(outputBuffer, 0, stagingBuffer, 0, totalPixels * 4);
    device.queue.submit([encoder.finish()]);

    await stagingBuffer.mapAsync(GPUMapMode.READ);
    const result = new Uint8ClampedArray(stagingBuffer.getMappedRange().slice(0));
    stagingBuffer.unmap();

    
    const canvas = canvasRef.current;
    if (!canvas) return;
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    const imgData = new ImageData(result, width, height);
    ctx.putImageData(imgData, 0, 0);

    
    uniformBuffer.destroy();
    pixelBuffer.destroy();
    outputBuffer.destroy();
    stagingBuffer.destroy();
  }, [rawData, width, height, dataMin, dataMax, shadow, midtone, highlight]);

  
  const renderCPU = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !rawData) return;
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    const imgData = ctx.createImageData(width, height);
    const data = imgData.data;

    const range = Math.max(dataMax - dataMin, 1e-20);
    const invRange = 1.0 / range;
    const clipRange = Math.max(highlight - shadow, 1e-10);

    const mtf = (x, m) => {
      if (x <= 0) return 0;
      if (x >= 1) return 1;
      return (m - 1) * x / ((2 * m - 1) * x - m);
    };

    for (let i = 0; i < rawData.length; i++) {
      const raw = rawData[i];
      let val = 0;
      if (raw === raw) { 
        const norm = Math.max(0, Math.min(1, (raw - dataMin) * invRange));
        const clipped = Math.max(0, Math.min(1, (norm - shadow) / clipRange));
        val = mtf(clipped, midtone);
      }
      const byte = Math.round(Math.max(0, Math.min(255, val * 255)));
      const off = i * 4;
      data[off] = byte;
      data[off + 1] = byte;
      data[off + 2] = byte;
      data[off + 3] = 255;
    }

    ctx.putImageData(imgData, 0, 0);
  }, [rawData, width, height, dataMin, dataMax, shadow, midtone, highlight]);

  return (
    <canvas
      ref={canvasRef}
      className={`max-w-full h-auto ${className}`}
      style={{ imageRendering: "pixelated" }}
    />
  );
}
