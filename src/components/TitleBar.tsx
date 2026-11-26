import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import iconUrl from "@/assets/app-icon.png";
import { isComposeDirty, useComposeStore } from "@/stores/compose.store";

const isMac = navigator.userAgent.includes("Macintosh");

export default function TitleBar() {
  const { t } = useTranslation();
  const appWindow = getCurrentWindow();

  async function handleCloseWindow() {
    if (isComposeDirty()) {
      useComposeStore.setState({ showComposeLeaveConfirm: true, pendingView: null,
        pendingNavigation: () => { void appWindow.close(); } });
      return;
    }
    await appWindow.close();
  }

  return (
    <div
      data-tauri-drag-region
      className={`titlebar flex items-center justify-between h-9 select-none${isMac ? " titlebar--mac" : ""}`}
    >
      {/* Brand block. On macOS the window uses the overlay title bar with the
          native title hidden (hiddenTitle) and the traffic lights at
          trafficLightPosition in tauri.conf.json: titlebar--mac reserves
          that space (see shell.css). */}
      <div data-tauri-drag-region className="flex items-center gap-2 px-3">
        <img
          data-tauri-drag-region
          src={iconUrl}
          alt=""
          aria-hidden="true"
          draggable={false}
          className="h-5 w-5 shrink-0 bg-transparent object-contain"
        />
        <span className="titlebar__brand">
          Pebble
        </span>
      </div>
      {!isMac && (
        <div className="flex items-center">
          <button
            type="button"
            onClick={() => appWindow.minimize()}
            className="titlebar-btn"
            aria-label={t("titleBar.minimize")}
          >
            <svg width="10" height="1" viewBox="0 0 10 1">
              <rect width="10" height="1" fill="currentColor" />
            </svg>
          </button>
          <button
            type="button"
            onClick={() => appWindow.toggleMaximize()}
            className="titlebar-btn"
            aria-label={t("titleBar.maximize")}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <rect
                width="9"
                height="9"
                x="0.5"
                y="0.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
              />
            </svg>
          </button>
          <button
            type="button"
            onClick={() => void handleCloseWindow()}
            className="titlebar-btn titlebar-btn--close"
            aria-label={t("titleBar.close")}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M1,1 L9,9 M9,1 L1,9" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </button>
        </div>
      )}
    </div>
  );
}
