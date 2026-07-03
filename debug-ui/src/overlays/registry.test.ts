import { describe, expect, it, vi } from "vitest";
import { identity } from "../viewport/transform";
import { createRegistry, type OverlayContext, type OverlayLayer } from "./registry";

/** A context whose `ctx`/`groundTruth`/`scan` fields are never touched by
 * the registry itself (only forwarded to each layer's `draw`), so a bare
 * stand-in is enough here — the fake-canvas layer tests exercise the real
 * shape. */
function fakeContext(): OverlayContext {
  return {
    ctx: {} as CanvasRenderingContext2D,
    view: identity,
    scan: null,
    groundTruth: null,
    imageSize: [100, 100],
    workingScale: 1,
  };
}

function fakeLayer(
  id: string,
  overrides: Partial<OverlayLayer> = {},
): OverlayLayer & { calls: number } {
  const layer = {
    id,
    label: id,
    defaultEnabled: true,
    calls: 0,
    draw() {
      layer.calls++;
    },
    ...overrides,
  };
  return layer;
}

describe("createRegistry", () => {
  it("seeds `enabled` from each layer's defaultEnabled", () => {
    const a = fakeLayer("a", { defaultEnabled: true });
    const b = fakeLayer("b", { defaultEnabled: false });
    const c = fakeLayer("c", { defaultEnabled: true });
    const registry = createRegistry([a, b, c]);
    expect(registry.enabled).toEqual(new Set(["a", "c"]));
  });

  it("exposes `layers` in registration order", () => {
    const a = fakeLayer("a");
    const b = fakeLayer("b");
    const registry = createRegistry([a, b]);
    expect(registry.layers).toEqual([a, b]);
  });

  describe("toggle", () => {
    it("disables an enabled layer and re-enables it", () => {
      const registry = createRegistry([fakeLayer("a", { defaultEnabled: true })]);
      expect(registry.enabled.has("a")).toBe(true);
      registry.toggle("a");
      expect(registry.enabled.has("a")).toBe(false);
      registry.toggle("a");
      expect(registry.enabled.has("a")).toBe(true);
    });

    it("enables a layer that started disabled", () => {
      const registry = createRegistry([fakeLayer("a", { defaultEnabled: false })]);
      expect(registry.enabled.has("a")).toBe(false);
      registry.toggle("a");
      expect(registry.enabled.has("a")).toBe(true);
    });
  });

  describe("drawAll", () => {
    it("draws only enabled layers, in registration order", () => {
      const order: string[] = [];
      const a = fakeLayer("a", {
        defaultEnabled: true,
        draw() {
          order.push("a");
        },
      });
      const b = fakeLayer("b", {
        defaultEnabled: false,
        draw() {
          order.push("b");
        },
      });
      const c = fakeLayer("c", {
        defaultEnabled: true,
        draw() {
          order.push("c");
        },
      });
      const registry = createRegistry([a, b, c]);
      registry.drawAll(fakeContext());
      expect(order).toEqual(["a", "c"]);
    });

    it("reflects toggle changes on the next drawAll", () => {
      const a = fakeLayer("a", { defaultEnabled: true });
      const registry = createRegistry([a]);
      registry.toggle("a");
      registry.drawAll(fakeContext());
      expect(a.calls).toBe(0);
    });

    it("passes the OverlayContext through to each layer unchanged", () => {
      const ctx = fakeContext();
      let received: OverlayContext | null = null;
      const a = fakeLayer("a", {
        draw(o) {
          received = o;
        },
      });
      createRegistry([a]).drawAll(ctx);
      expect(received).toBe(ctx);
    });

    it("a throwing layer does not stop the rest from drawing", () => {
      const b = fakeLayer("b");
      const throwing = fakeLayer("bad", {
        draw() {
          throw new Error("boom");
        },
      });
      const registry = createRegistry([throwing, b]);
      const spy = vi.spyOn(console, "error").mockImplementation(() => {});
      expect(() => registry.drawAll(fakeContext())).not.toThrow();
      expect(b.calls).toBe(1);
      spy.mockRestore();
    });

    it("logs a throwing layer's error only once, not on every drawAll", () => {
      const throwing = fakeLayer("bad", {
        draw() {
          throw new Error("boom");
        },
      });
      const registry = createRegistry([throwing]);
      const spy = vi.spyOn(console, "error").mockImplementation(() => {});
      registry.drawAll(fakeContext());
      registry.drawAll(fakeContext());
      registry.drawAll(fakeContext());
      expect(spy).toHaveBeenCalledTimes(1);
      spy.mockRestore();
    });

    it("logs distinct throwing layers independently", () => {
      const bad1 = fakeLayer("bad1", {
        draw() {
          throw new Error("boom1");
        },
      });
      const bad2 = fakeLayer("bad2", {
        draw() {
          throw new Error("boom2");
        },
      });
      const registry = createRegistry([bad1, bad2]);
      const spy = vi.spyOn(console, "error").mockImplementation(() => {});
      registry.drawAll(fakeContext());
      expect(spy).toHaveBeenCalledTimes(2);
      spy.mockRestore();
    });
  });
});
