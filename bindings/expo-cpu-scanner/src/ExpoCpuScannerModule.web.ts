/**
 * Web stub — the browser path uses `qr-lab-wasm` via the debug UI, not this
 * Expo module. Calling into native methods on web throws clearly.
 */
function unsupported(): never {
  throw new Error(
    "expo-cpu-scanner has no web native module; use qr-lab-wasm / the debug UI on web",
  );
}

export default {
  scanLuma: unsupported,
  destroyScanner: () => {},
};
