/** OpenCV lens conventions, pixel centers at integer coordinates. */
export interface Intrinsics {
  fx: number;
  fy: number;
  cx: number;
  cy: number;
}

export interface CameraCalibration {
  intrinsics: Intrinsics;
  distortion:
    | { model: "none" }
    | { model: "fisheye"; coefficients: [number, number, number, number] }
    | { model: "brown_conrady"; coefficients: [number, number, number, number, number] };
  rectified_intrinsics?: Intrinsics;
}
