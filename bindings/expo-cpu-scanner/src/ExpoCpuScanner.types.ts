/** One module-region corner `[x, y]` in the coordinate space noted on the field. */
export type QrCorner = [x: number, y: number];

export type QrCodeDetection = {
  payload: string;
  /** Base64 of the raw payload bytes. */
  payloadBytesB64: string;
  version: number;
  /** ECC level: `L` / `M` / `Q` / `H` / `?`. */
  ecc: string;
  mirrored: boolean;
  inverted: boolean;
  dimension: number;
  /** TL / TR / BR / BL corners in **working** pixels. */
  corners: [QrCorner, QrCorner, QrCorner, QrCorner];
  /** Subpixel-refined corners in **source** pixels when `refine` was true. */
  refinedCorners: [QrCorner, QrCorner, QrCorner, QrCorner] | null;
};

export type QrScanTimings = {
  tilesNs: number;
  findersNs: number;
  tripletsNs: number;
  versionNs: number;
  alignmentNs: number;
  sampleDecodeNs: number;
  refineNs: number;
};

export type QrScanResult = {
  scanWidth: number;
  scanHeight: number;
  /** `working / source` (width axis). Working → source: `working / sourceScale`. */
  sourceScale: number;
  codes: QrCodeDetection[];
  timings: QrScanTimings;
};

export type ScanLumaOptions = {
  /** Cap on the working view's longest side; `0` = no cap (full source). */
  maxDim?: number;
  /** Enable subpixel corner refinement (populates `refinedCorners`). */
  refine?: boolean;
};
