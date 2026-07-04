import { useState } from "react";
import type { OverlayRegistry } from "../overlays/registry";

export interface LayerPanelProps {
  registry: OverlayRegistry;
  /** Called after every toggle, with the toggled layer's `id`. Optional —
   * added in Task 6 so `App.tsx` can force a `Viewport` redraw: toggling a
   * layer mutates `registry.enabled` in place without changing `registry`
   * or the `overlays` callback's identity, and `Viewport` only redraws on
   * an `image`/`overlays` identity change (or a pan/zoom/resize), so
   * without this hook a toggle wouldn't visibly repaint the canvas until
   * some unrelated interaction did. */
  onToggle?: (id: string) => void;
}

/**
 * Checkbox list mirroring `registry.enabled`, one row per registered
 * layer in registration order. `registry.enabled` is a `Set` mutated in
 * place by `toggle` (not React state), so this component can't just read
 * it and rely on React to notice a change — it forces its own re-render
 * with a local counter after every toggle it triggers. Kept intentionally
 * dumb: this is the only place layers get toggled from, so there's no
 * need for a prop-driven "version" to sync against external toggles.
 */
export function LayerPanel({ registry, onToggle }: LayerPanelProps) {
  const [, forceRender] = useState(0);

  const handleToggle = (id: string) => {
    registry.toggle(id);
    forceRender((n) => n + 1);
    onToggle?.(id);
  };

  return (
    <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
      {registry.layers.map((layer) => (
        <li key={layer.id}>
          <label style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={registry.enabled.has(layer.id)}
              onChange={() => handleToggle(layer.id)}
            />
            {layer.label}
          </label>
        </li>
      ))}
    </ul>
  );
}
