// Run in the debug UI's browser console:
// await (await import('/scripts/check-scanner.browser.ts')).checkScanner()
import { ScannerClient } from "../src/scanner/client";

export async function checkScanner() {
  const checks: string[] = [];
  const blobUrls: string[] = [];
  const clients: ScannerClient[] = [];
  const create = (script?: string) => {
    let url = "/src/scanner/worker.ts";
    if (script !== undefined) {
      url = URL.createObjectURL(new Blob([script], { type: "text/javascript" }));
      blobUrls.push(url);
    }
    const client = new ScannerClient(new Worker(url, { type: "module" }));
    clients.push(client);
    return client;
  };
  const rejected = async (promises: Promise<unknown>[]) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const results = await Promise.race([
        Promise.allSettled(promises),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error("requests did not settle")), 2000);
        }),
      ]);
      if (results.some(result => result.status !== "rejected")) throw new Error("request unexpectedly resolved");
      return results.map(result => result.status === "rejected" ? String(result.reason) : "");
    } finally { clearTimeout(timer); }
  };
  try {
    // Exercise the actual worker's initialization catch, including its import.
    const initFailure = create(`globalThis.fetch = () => Promise.reject(new Error("injected WASM load failure"));
      await import(${JSON.stringify(new URL("/src/scanner/worker.ts", location.href).href)});`);
    const initErrors = await rejected([initFailure.init()]);
    if (!initErrors[0]?.includes("injected WASM load failure")) throw new Error(initErrors.join(", "));
    checks.push("real WASM initialization failure rejects immediately");

    const options = { maxDim: 0, withTrace: false, refine: true };
    const crash = create('postMessage({type:"ready"}); onmessage = () => { throw new Error("injected runtime crash"); };');
    await crash.init();
    await rejected([crash.scan(new Uint8ClampedArray(16), 2, 2, options),
      crash.scan(new Uint8ClampedArray(16), 2, 2, options), crash.robustPresets()]);
    checks.push("real worker crash rejects in-flight, queued and preset requests");

    const disposed = create('onmessage = () => {};');
    const pending = [disposed.init(), disposed.scan(new Uint8ClampedArray(16), 2, 2, options),
      disposed.scan(new Uint8ClampedArray(16), 2, 2, options), disposed.robustPresets()];
    disposed.dispose();
    await rejected(pending);
    checks.push("disposing a real worker settles every pending request");

    const client = create();
    await client.init();
    const bitmap = await createImageBitmap(await (await fetch('/fixtures/near_00.png')).blob());
    const { width, height } = bitmap;
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d")!;
    context.drawImage(bitmap, 0, 0);
    bitmap.close();
    const rgba = context.getImageData(0, 0, width, height).data;
    const truth = await (await fetch('/fixtures/near_00.json')).json();
    const verify = (payloads: string[]) => {
      for (const code of truth.codes) {
        if (!payloads.includes(code.payload)) throw new Error(`missing payload: ${code.payload}`);
      }
    };
    verify((await client.scan(rgba, width, height, options)).result.detections.codes.map(code => code.payload));
    checks.push("classic scan decodes near_00");
    const presets = await client.robustPresets();
    verify((await client.scanRobust(rgba, width, height,
      { maxDim: 1280, refine: true, config: presets.robustFast, capture: true }))
      .result.detections.codes.map(code => code.payload));
    checks.push("robust scan decodes near_00 with capture");
    client.configureSession(presets.robustFast, 3, 4);
    verify((await client.scanSessionFrame(rgba, width, height, { maxDim: 1280, refine: true }))
      .result.detections.codes.map(code => code.payload));
    client.resetSession();
    verify((await client.scanSessionFrame(rgba, width, height, { maxDim: 1280, refine: true }))
      .result.detections.codes.map(code => code.payload));
    checks.push("session scan and reset decode near_00");
    if (rgba.byteLength !== width * height * 4) throw new Error("caller buffer was detached");
    return checks;
  } finally {
    for (const client of clients) client.dispose();
    for (const url of blobUrls) URL.revokeObjectURL(url);
  }
}
