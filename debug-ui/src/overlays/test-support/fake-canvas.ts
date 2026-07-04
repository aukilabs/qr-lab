// Shared fake `CanvasRenderingContext2D` for the layer tests
// (layers/*.test.ts): records every drawing *method* call (not style
// property writes — `fillStyle`/`strokeStyle`/`lineWidth`/`font` are
// plain mutable fields, uninteresting for a primitive-count assertion) so
// a test can assert exactly what a layer drew without a real DOM/canvas,
// which vitest's `node` test environment doesn't provide.

export interface RecordedCall {
  name: string;
  args: unknown[];
}

export interface FakeCanvas {
  ctx: CanvasRenderingContext2D;
  calls: RecordedCall[];
  /** Calls whose `name` matches, in order — the usual thing a test wants
   * (e.g. `callsNamed("fillRect").length`). */
  callsNamed(name: string): RecordedCall[];
}

const RECORDED_METHODS = [
  "beginPath",
  "closePath",
  "moveTo",
  "lineTo",
  "arc",
  "rect",
  "stroke",
  "fill",
  "fillRect",
  "strokeRect",
  "fillText",
  "strokeText",
  "save",
  "restore",
  "setLineDash",
  "translate",
  "rotate",
  "scale",
  "clip",
] as const;

export function createFakeCanvas(): FakeCanvas {
  const calls: RecordedCall[] = [];
  const ctx = {} as Record<string, unknown>;

  for (const name of RECORDED_METHODS) {
    ctx[name] = (...args: unknown[]) => {
      calls.push({ name, args });
    };
  }

  // Plain read/write style fields a layer may set; not recorded as calls.
  ctx.fillStyle = "";
  ctx.strokeStyle = "";
  ctx.lineWidth = 1;
  ctx.font = "";
  ctx.textAlign = "start";
  ctx.textBaseline = "alphabetic";
  ctx.globalAlpha = 1;

  return {
    ctx: ctx as unknown as CanvasRenderingContext2D,
    calls,
    callsNamed(name: string) {
      return calls.filter((c) => c.name === name);
    },
  };
}
