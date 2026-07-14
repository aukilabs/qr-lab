import { requireNativeViewManager } from "expo-modules-core";
import type { FrameCameraViewProps } from "./FrameCamera.types";

// Module Name("FrameCamera") — requireNativeViewManager takes the module name
// (same pattern as ExpoDomWebViewModule / ExpoLinearGradient).
const NativeView = requireNativeViewManager<FrameCameraViewProps>("FrameCamera");

export function FrameCameraView(props: FrameCameraViewProps) {
  return <NativeView {...props} />;
}
