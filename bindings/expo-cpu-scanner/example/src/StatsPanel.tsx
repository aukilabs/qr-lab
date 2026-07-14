import { Platform, StyleSheet, Text, View } from "react-native";
import type { QrScanResult } from "expo-cpu-scanner";

type Props = {
  result: QrScanResult | null;
  wallMs: number | null;
  fps: number;
  frameSize: { width: number; height: number } | null;
  error: string | null;
  scanning: boolean;
};

function formatNs(ns: number): string {
  if (ns <= 0) return "—";
  if (ns < 1_000_000) return `${(ns / 1_000).toFixed(0)}µs`;
  return `${(ns / 1_000_000).toFixed(1)}ms`;
}

/** Floating telemetry window over the camera feed. */
export function StatsPanel({
  result,
  wallMs,
  fps,
  frameSize,
  error,
  scanning,
}: Props) {
  const t = result?.timings;
  return (
    <View style={styles.panel}>
      <View style={styles.header}>
        <View style={styles.dot} />
        <Text style={styles.title}>QRK LIVE</Text>
        <Text style={styles.badge}>{scanning ? "SCAN" : "IDLE"}</Text>
      </View>

      <Row label="feed" value={`${fps.toFixed(1)} fps`} />
      <Row
        label="frame"
        value={frameSize ? `${frameSize.width}×${frameSize.height}` : "—"}
      />
      <Row
        label="working"
        value={result ? `${result.scanWidth}×${result.scanHeight}` : "—"}
      />
      <Row label="wall" value={wallMs != null ? `${wallMs.toFixed(1)}ms` : "—"} />
      <Row label="codes" value={result ? String(result.codes.length) : "—"} />

      {t && (
        <>
          <View style={styles.divider} />
          <Row label="tiles" value={formatNs(t.tilesNs)} />
          <Row label="finders" value={formatNs(t.findersNs)} />
          <Row label="triplets" value={formatNs(t.tripletsNs)} />
          <Row label="sample" value={formatNs(t.sampleDecodeNs)} />
          <Row label="refine" value={formatNs(t.refineNs)} />
        </>
      )}

      {result && result.codes.length > 0 && (
        <>
          <View style={styles.divider} />
          {result.codes.slice(0, 3).map((c, i) => (
            <Text key={i} style={styles.payload} numberOfLines={1}>
              {c.payload}
            </Text>
          ))}
        </>
      )}

      {error && <Text style={styles.error}>{error}</Text>}
    </View>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.row}>
      <Text style={styles.label}>{label}</Text>
      <Text style={styles.value}>{value}</Text>
    </View>
  );
}

const mono = Platform.select({
  ios: "Menlo",
  android: "monospace",
  default: "monospace",
});

const styles = StyleSheet.create({
  panel: {
    position: "absolute",
    top: 56,
    right: 12,
    width: 168,
    backgroundColor: "rgba(14, 18, 25, 0.88)",
    borderColor: "#243044",
    borderWidth: 1,
    borderRadius: 2,
    padding: 10,
    gap: 3,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    marginBottom: 6,
  },
  dot: {
    width: 6,
    height: 6,
    backgroundColor: "#3de0c5",
  },
  title: {
    color: "#e8edf5",
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
    flex: 1,
  },
  badge: {
    color: "#3de0c5",
    fontSize: 9,
    fontFamily: mono,
    letterSpacing: 0.5,
  },
  row: {
    flexDirection: "row",
    justifyContent: "space-between",
  },
  label: {
    color: "#6b7c91",
    fontSize: 10,
    fontFamily: mono,
  },
  value: {
    color: "#e8edf5",
    fontSize: 10,
    fontFamily: mono,
    fontVariant: ["tabular-nums"],
  },
  divider: {
    height: 1,
    backgroundColor: "#243044",
    marginVertical: 4,
  },
  payload: {
    color: "#3de0c5",
    fontSize: 10,
    fontFamily: mono,
  },
  error: {
    color: "#f07178",
    fontSize: 10,
    marginTop: 4,
    fontFamily: mono,
  },
});
