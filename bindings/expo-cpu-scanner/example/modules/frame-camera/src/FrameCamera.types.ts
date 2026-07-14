export type FrameCameraFrameEvent = {
  /** Frame width in pixels. */
  width: number;
  /** Frame height in pixels. */
  height: number;
  /** Row stride of the Y plane in bytes (≥ width). */
  stride: number;
  /**
   * Luma payload. When `encoding === "base64"` this is a base64 string of the
   * tight Y plane; otherwise a Uint8Array (or bridge binary string).
   */
  luma: Uint8Array | string;
  /** How `luma` is encoded. Android sends `"base64"`. */
  encoding?: "base64" | "raw";
  /** Monotonic frame id (native counter). */
  frameId: number;
  /** Capture timestamp in milliseconds (native clock, best-effort). */
  timestampMs: number;
};

import type { StyleProp, ViewStyle } from "react-native";

export type FrameCameraViewProps = {
  /** Target analysis rate. Native drops intermediate frames. Default 12. */
  targetFps?: number;
  /** When false, camera stays open for preview but no frames are emitted. */
  active?: boolean;
  style?: StyleProp<ViewStyle>;
  onFrame?: (event: { nativeEvent: FrameCameraFrameEvent }) => void;
  onError?: (event: { nativeEvent: { message: string } }) => void;
};
