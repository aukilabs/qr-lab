import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Placeholder root — Plan 3's later tasks replace this with the real
// viewport/overlay/panel app (see docs/superpowers/plans for the shape).
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <div>qrk debug ui</div>
  </StrictMode>,
);
