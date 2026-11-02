// Evaluate this expression with Argent debugger-evaluate after the example
// connects to Metro on an iOS simulator. Calls the actual Swift/C bridge.
(() => {
  const scanner = globalThis.expo?.modules?.ExpoCpuScanner;
  if (!scanner) throw new Error("Open the scanner example before running this check");
  const results = [];
  const invalid = [
    ["width overflow", 2 ** 32, 1, 2 ** 32, 0],
    ["height overflow", 1, 2 ** 32, 1, 0],
    ["stride overflow", 1, 1, 2 ** 32, 0],
    ["maxDim overflow", 1, 1, 1, 2 ** 32],
    ["byte-count overflow", 1, 2 ** 32 - 1, 2 ** 32 - 1, 0],
    ["zero width", 0, 1, 1, 0],
    ["short stride", 2, 1, 1, 0],
    ["short buffer", 16, 16, 16, 0],
  ];
  for (const [name, width, height, stride, maxDim] of invalid) {
    try {
      scanner.scanLuma(new Uint8Array(8), width, height, stride, maxDim, true);
      throw new Error(`accepted invalid input: ${name}`);
    } catch (error) {
      if (!String(error).includes("invalid luma input")) throw error;
      results.push(`${name}: rejected`);
    }
  }
  const blank = JSON.parse(scanner.scanLuma(new Uint8Array(64).fill(255), 8, 8, 8, 0, true));
  if (blank.scanWidth !== 8 || blank.scanHeight !== 8 || blank.codes.length !== 0) {
    throw new Error("Valid scan failed after invalid inputs");
  }
  results.push("valid scan after invalid inputs: passed");
  // Padding is needed between rows, but not after the final row. Negative
  // maxDim retains the bridge's existing full-resolution behavior.
  const strided = JSON.parse(scanner.scanLuma(new Uint8Array(12).fill(255), 4, 2, 8, -1, false));
  if (strided.scanWidth !== 4 || strided.scanHeight !== 2 || strided.codes.length !== 0) {
    throw new Error("Strided scan failed");
  }
  results.push("strided input with unpadded final row and negative maxDim: passed");
  return results;
})()
