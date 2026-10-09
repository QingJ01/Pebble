import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import iconUrl from "@/assets/app-icon.png";
import { isComposeDirty } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import i18n from "@/lib/i18n";

const isMac = navigator.userAgent.includes("Macintosh");

export default function TitleBar() {
  const { t } = useTranslation();
  const appWindow = getCurrentWindow();

  async function handleCloseWindow() {
    if (isComposeDirty()) {
      const confirmed = await useConfirmStore.getState().confirm({
        title: i18n.t("compose.discardDraft", "Discard draft"),
        message: i18n.t("compose.discardDraftConfirm", "You have an unsaved draft. Discard and leave?"),
        destructive: true,
      });
      if (!confirmed) return;
    }
    await appWindow.close();
  }

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between h-9 select-none"
      style={{ backgroundColor: "var(--color-titlebar-bg)" }}
    >
      {/* Brand block under the native titlebar; the macOS traffic lights sit at
          the top-left corner via trafficLightPosition in tauri.conf.json. */}
      <div data-tauri-drag-region className="flex items-center gap-2 px-3">
        <img
          data-tauri-drag-region
          src={iconUrl}
          alt=""
          aria-hidden="true"
          draggable={false}
          className="h-5 w-5 shrink-0 bg-transparent object-contain"
        />
        <span
          className="text-sm font-semibold"
          style={{ color: "var(--color-text-primary)" }}
        >
          Pebble
        </span>
      </div>
      {!isMac && (
        <div className="flex items-center">
          <button
            onClick={() => appWindow.minimize()}
            className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
            aria-label={t("titleBar.minimize")}
          >
            <svg width="10" height="1" viewBox="0 0 10 1">
              <rect width="10" height="1" fill="currentColor" />
            </svg>
          </button>
          <button
            onClick={() => appWindow.toggleMaximize()}
            className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
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
            onClick={() => void handleCloseWindow()}
            className="h-9 w-11 inline-flex items-center justify-center hover:bg-red-500 hover:text-white"
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
