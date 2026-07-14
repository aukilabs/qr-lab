import { useCallback, useEffect, useRef, useState } from "react";
import {
  LayoutChangeEvent,
  PermissionsAndroid,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { scanLuma, type QrScanResult } from "expo-cpu-scanner";
import { FrameCameraView, type FrameCameraFrameEvent } from "frame-camera";
import { ScanOverlay } from "./src/ScanOverlay";
import { StatsPanel } from "./src/StatsPanel";

// Scan rate (preview is free-running hardware on native). Bridge payload is
// base64 Y up to EMIT_MAX≈1080 on Android — keep fps modest to avoid OOM.
const TARGET_FPS = 6;
const MAX_DIM = 1080;
const REFINE = true;

async function ensureCameraPermission(): Promise<boolean> {
  if (Platform.OS === "android") {
    const result = await PermissionsAndroid.request(
      PermissionsAndroid.PERMISSIONS.CAMERA,
      {
        title: "Camera permission",
        message: "QRK example needs the camera to scan live frames.",
        buttonPositive: "OK",
      },
    );
    return result === PermissionsAndroid.RESULTS.GRANTED;
  }
  // iOS: FrameCamera requests access when the session starts.
  return true;
}

/** Decode standard base64. Only call when native tags encoding === "base64". */
function base64ToUint8(b64: string): Uint8Array {
  let s = b64.trim();
  const comma = s.indexOf(",");
  if (s.startsWith("data:") && comma >= 0) s = s.slice(comma + 1);
  // Drop any non-alphabet noise the bridge might inject.
  s = s.replace(/[^A-Za-z0-9+/=]/g, "");
  const pad = s.length % 4;
  if (pad) s = s + "=".repeat(4 - pad);

  if (typeof atob === "function") {
    const bin = atob(s);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  }
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const B = (globalThis as any).Buffer;
  if (B) return new Uint8Array(B.from(s, "base64"));
  throw new Error("no base64 decoder available");
}

/** Latin-1 / binary string → bytes (Expo sometimes delivers byte[] this way). */
function binaryStringToUint8(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

function toUint8(data: unknown, encoding?: string): Uint8Array {
  if (data instanceof Uint8Array) return data;
  if (ArrayBuffer.isView(data)) {
    const v = data as ArrayBufferView;
    return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
  }
  if (data instanceof ArrayBuffer) return new Uint8Array(data);
  if (typeof data === "string") {
    // Only base64-decode when native explicitly tags it. Auto-detect was
    // mis-decoding binary strings → "invalid character" / length errors.
    if (encoding === "base64") {
      return base64ToUint8(data);
    }
    return binaryStringToUint8(data);
  }
  if (Array.isArray(data)) return Uint8Array.from(data as number[]);
  throw new Error(`unexpected luma type: ${typeof data}`);
}

export default function App() {
  const [permitted, setPermitted] = useState<boolean | null>(null);
  const [result, setResult] = useState<QrScanResult | null>(null);
  const [wallMs, setWallMs] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [frameSize, setFrameSize] = useState<{ width: number; height: number } | null>(
    null,
  );
  const [viewSize, setViewSize] = useState({ width: 0, height: 0 });
  const [fps, setFps] = useState(0);
  const [paused, setPaused] = useState(false);

  const busyRef = useRef(false);
  const fpsWindowRef = useRef<number[]>([]);
  const lastFrameIdRef = useRef(-1);

  useEffect(() => {
    let cancelled = false;
    ensureCameraPermission().then((ok) => {
      if (!cancelled) setPermitted(ok);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const onLayout = useCallback((e: LayoutChangeEvent) => {
    const { width, height } = e.nativeEvent.layout;
    setViewSize({ width, height });
  }, []);

  const onFrame = useCallback((event: { nativeEvent: FrameCameraFrameEvent }) => {
    const frame = event.nativeEvent;
    if (frame.frameId === lastFrameIdRef.current) return;
    lastFrameIdRef.current = frame.frameId;

    // FPS estimate from emit timestamps (JS side).
    const now = performance.now();
    const w = fpsWindowRef.current;
    w.push(now);
    while (w.length > 0 && now - w[0]! > 1000) w.shift();
    setFps(w.length);

    setFrameSize({ width: frame.width, height: frame.height });

    if (busyRef.current || paused) return;
    busyRef.current = true;

    // Yield to next microtask so the native emitter can return promptly.
    queueMicrotask(() => {
      try {
        const luma = toUint8(frame.luma, frame.encoding);
        const t0 = performance.now();
        const scanned = scanLuma(luma, frame.width, frame.height, {
          maxDim: MAX_DIM,
          refine: REFINE,
          stride: frame.stride,
        });
        const elapsed = performance.now() - t0;
        setResult(scanned);
        setWallMs(elapsed);
        setError(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        busyRef.current = false;
      }
    });
  }, [paused]);

  const onCameraError = useCallback((event: { nativeEvent: { message: string } }) => {
    setError(event.nativeEvent.message);
  }, []);

  if (permitted === null) {
    return (
      <View style={styles.center}>
        <Text style={styles.muted}>Requesting camera…</Text>
      </View>
    );
  }

  if (!permitted) {
    return (
      <View style={styles.center}>
        <Text style={styles.error}>Camera permission denied</Text>
        <Text style={styles.muted}>Enable it in system settings and relaunch.</Text>
      </View>
    );
  }

  return (
    <View style={styles.root} onLayout={onLayout}>
      {/* Avoid expo-status-bar / RCTStatusBarManager — requires
          UIViewControllerBasedStatusBarAppearance=NO in Info.plist. */}

      <FrameCameraView
        style={StyleSheet.absoluteFill}
        targetFps={TARGET_FPS}
        active={!paused}
        onFrame={onFrame}
        onError={onCameraError}
      />

      <ScanOverlay
        result={result}
        frameWidth={frameSize?.width ?? 0}
        frameHeight={frameSize?.height ?? 0}
        viewWidth={viewSize.width}
        viewHeight={viewSize.height}
      />

      <StatsPanel
        result={result}
        wallMs={wallMs}
        fps={fps}
        frameSize={frameSize}
        error={error}
        scanning={!paused && !busyRef.current}
      />

      <View style={styles.bottomBar}>
        <Text style={styles.hint}>
          frame-camera → scanLuma · maxDim {MAX_DIM} · refine {REFINE ? "on" : "off"}
        </Text>
        <Pressable
          style={[styles.btn, paused && styles.btnActive]}
          onPress={() => setPaused((p) => !p)}
        >
          <Text style={styles.btnText}>{paused ? "Resume" : "Pause"}</Text>
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: "#000",
  },
  center: {
    flex: 1,
    backgroundColor: "#07090d",
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    gap: 8,
  },
  muted: {
    color: "#6b7c91",
    fontSize: 13,
    textAlign: "center",
  },
  error: {
    color: "#f07178",
    fontSize: 14,
    fontWeight: "600",
  },
  bottomBar: {
    position: "absolute",
    left: 12,
    right: 12,
    bottom: 36,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  hint: {
    flex: 1,
    color: "#9aabbf",
    fontSize: 11,
    fontFamily: Platform.select({ ios: "Menlo", android: "monospace", default: "monospace" }),
  },
  btn: {
    borderWidth: 1,
    borderColor: "#243044",
    backgroundColor: "rgba(14, 18, 25, 0.9)",
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 2,
  },
  btnActive: {
    borderColor: "#3de0c5",
    backgroundColor: "rgba(61, 224, 197, 0.14)",
  },
  btnText: {
    color: "#e8edf5",
    fontSize: 13,
    fontWeight: "600",
  },
});
