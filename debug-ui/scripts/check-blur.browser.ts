// Run in the debug UI's browser console:
// await (await import('/scripts/check-blur.browser.ts')).checkBlur()
// Uses real canvas filters/readback; no DOM mocks or extra dependencies.
import { GaussianBlurCanvas, mulberry32 } from "../src/scene3d/camSim";

function canvas(width: number, height: number): HTMLCanvasElement {
  return Object.assign(document.createElement("canvas"), { width, height });
}

// Previous implementation, retained as an independent pixel/timing reference.
function reference(rgba: Uint8ClampedArray, width: number, height: number, sigma: number) {
  const source = canvas(width, height);
  source.getContext("2d")!.putImageData(new ImageData(rgba.slice(), width, height), 0, 0);
  const destination = canvas(width, height);
  const context = destination.getContext("2d")!;
  context.filter = `blur(${sigma}px)`;
  context.drawImage(source, 0, 0);
  return context.getImageData(0, 0, width, height).data;
}

export function checkBlur() {
  let allocations = 0;
  const blur = new GaussianBlurCanvas((width, height) => {
    allocations++;
    return canvas(width, height);
  });
  const random = mulberry32(7);
  const checks = [];
  // Repeated sizes, transparency, sigma changes and non-square resizing
  // catch old-frame accumulation and context state lost during resizing.
  for (const [width, height, sigma, transparent] of [
    [64, 48, 1.5, 0], [64, 48, 2, 1], [64, 48, 0.5, 1],
    [96, 32, 1, 0], [64, 48, 1.5, 0],
  ]) {
    const storage = new Uint8ClampedArray(width! * height! * 4 + 8);
    const input = storage.subarray(4, storage.length - 4);
    for (let i = 0; i < input.length; i++) {
      input[i] = i % 4 === 3 && !transparent ? 255 : Math.floor(random() * 256);
    }
    const before = input.slice();
    const expected = reference(input, width!, height!, sigma!);
    const actual = blur.apply(input, width!, height!, sigma!);
    let differingBytes = 0;
    for (let i = 0; i < input.length; i++) {
      if (actual[i] !== expected[i]) differingBytes++;
      if (input[i] !== before[i]) throw new Error("blur mutated its input");
    }
    if (differingBytes) throw new Error(`${width}x${height}: ${differingBytes} differing bytes`);
    // Returned frames must remain independent of subsequent calls.
    blur.apply(new Uint8ClampedArray(input.length), width!, height!, sigma!);
    if (actual.some((value, i) => value !== expected[i])) throw new Error("blur overwrote prior output");
    checks.push({ width, height, sigma, transparent: !!transparent, differingBytes });
  }
  if (allocations !== 2) throw new Error(`Expected 2 canvas allocations, got ${allocations}`);

  const timings = [];
  for (const size of [640, 960, 1280]) {
    const input = new Uint8ClampedArray(size * size * 4).fill(128);
    for (let i = 3; i < input.length; i += 4) input[i] = 255;
    const cached = new GaussianBlurCanvas(canvas);
    const oldTimes: number[] = [], newTimes: number[] = [];
    for (let i = 0; i < 35; i++) {
      // Alternate execution order to reduce systematic warmup/load bias.
      const oldRun = () => { const start = performance.now(); reference(input, size, size, 1.5); return performance.now() - start; };
      const newRun = () => { const start = performance.now(); cached.apply(input, size, size, 1.5); return performance.now() - start; };
      let oldMs, newMs;
      if (i % 2) { newMs = newRun(); oldMs = oldRun(); }
      else { oldMs = oldRun(); newMs = newRun(); }
      if (i >= 5) { oldTimes.push(oldMs); newTimes.push(newMs); }
    }
    const median = (values: number[]) => values.sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
    timings.push({ size, beforeMs: median(oldTimes), afterMs: median(newTimes) });
  }
  return { checks, canvasesForTenFrames: allocations, timings, userAgent: navigator.userAgent };
}
