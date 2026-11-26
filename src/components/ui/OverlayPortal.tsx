/**
 * OverlayPortal — render children into document.body, outside `.app-shell`
 * (which uses `isolation: isolate` and may sit under transformed ancestors).
 * Combine with the z-index tokens: --z-menu (menus/popovers), --z-modal
 * (dialogs), --z-nested (dialog opened from a dialog), --z-toast.
 *
 *   <OverlayPortal><div className="overlay-backdrop">…</div></OverlayPortal>
 */
import type { ReactNode } from "react";
import { createPortal } from "react-dom";

export default function OverlayPortal({ children }: { children: ReactNode }) {
  if (typeof document === "undefined") return null;
  return createPortal(children, document.body);
}
