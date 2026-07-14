import { StyleSheet, Text, View } from "react-native";
import type { QrCodeDetection, QrScanResult } from "expo-cpu-scanner";
import { mapFramePointToView, workingToSource } from "./mapFrameToView";

type Props = {
  result: QrScanResult | null;
  frameWidth: number;
  frameHeight: number;
  viewWidth: number;
  viewHeight: number;
};

function cornersForCode(
  code: QrCodeDetection,
  sourceScale: number,
): [number, number][] {
  if (code.refinedCorners) {
    return code.refinedCorners;
  }
  return code.corners.map((c) => workingToSource(c, sourceScale));
}

/**
 * Draws detection quads + payload chips on top of the camera preview.
 * Prefer refined (source-px) corners when present.
 */
export function ScanOverlay({
  result,
  frameWidth,
  frameHeight,
  viewWidth,
  viewHeight,
}: Props) {
  if (!result || result.codes.length === 0 || viewWidth <= 0) {
    return null;
  }

  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="none">
      {result.codes.map((code, i) => {
        const pts = cornersForCode(code, result.sourceScale).map(([x, y]) =>
          mapFramePointToView(x, y, frameWidth, frameHeight, viewWidth, viewHeight),
        );
        if (pts.length < 4) return null;

        // Axis-aligned bounding box for a simple chip; edges as thin views
        // between consecutive corners (no SVG dependency).
        const xs = pts.map((p) => p.x);
        const ys = pts.map((p) => p.y);
        const minX = Math.min(...xs);
        const maxX = Math.max(...xs);
        const minY = Math.min(...ys);
        const maxY = Math.max(...ys);

        // Edge segments: center-origin rotation (RN has no transformOrigin).
        const edges: {
          key: string;
          left: number;
          top: number;
          width: number;
          rotate: number;
        }[] = [];
        for (let e = 0; e < 4; e++) {
          const a = pts[e]!;
          const b = pts[(e + 1) % 4]!;
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const len = Math.hypot(dx, dy);
          const angle = (Math.atan2(dy, dx) * 180) / Math.PI;
          const midX = (a.x + b.x) / 2;
          const midY = (a.y + b.y) / 2;
          edges.push({
            key: `${i}-e${e}`,
            left: midX - len / 2,
            top: midY - 1,
            width: len,
            rotate: angle,
          });
        }

        return (
          <View key={i}>
            {edges.map((edge) => (
              <View
                key={edge.key}
                style={{
                  position: "absolute",
                  left: edge.left,
                  top: edge.top,
                  width: edge.width,
                  height: 2,
                  backgroundColor: "#3de0c5",
                  shadowColor: "#3de0c5",
                  shadowOpacity: 0.9,
                  shadowRadius: 4,
                  transform: [{ rotate: `${edge.rotate}deg` }],
                }}
              />
            ))}
            {pts.map((p, pi) => (
              <View
                key={`${i}-c${pi}`}
                style={{
                  position: "absolute",
                  left: p.x - 4,
                  top: p.y - 4,
                  width: 8,
                  height: 8,
                  borderRadius: 1,
                  backgroundColor: "#3de0c5",
                  borderWidth: 1,
                  borderColor: "#0a1210",
                }}
              />
            ))}
            <View
              style={{
                position: "absolute",
                left: Math.max(4, minX),
                top: Math.max(4, minY - 28),
                maxWidth: Math.max(80, maxX - minX + 8),
                backgroundColor: "rgba(7, 9, 13, 0.82)",
                borderColor: "#3de0c5",
                borderWidth: 1,
                paddingHorizontal: 6,
                paddingVertical: 2,
              }}
            >
              <Text style={styles.chip} numberOfLines={1}>
                {code.payload}
              </Text>
              <Text style={styles.chipMeta} numberOfLines={1}>
                v{code.version} · {code.ecc}
                {code.refinedCorners ? " · refined" : ""}
              </Text>
            </View>
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  chip: {
    color: "#3de0c5",
    fontSize: 11,
    fontWeight: "700",
  },
  chipMeta: {
    color: "#6b7c91",
    fontSize: 9,
    fontVariant: ["tabular-nums"],
  },
});
