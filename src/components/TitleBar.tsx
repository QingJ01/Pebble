import { getCurrentWindow } from "@tauri-apps/api/window";

export default function TitleBar() {
  const appWindow = getCurrentWindow();

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between h-9 select-none"
      style={{ backgroundColor: "var(--color-titlebar-bg)" }}
    >
      <div data-tauri-drag-region className="flex items-center gap-2 px-3">
        <span
          className="text-sm font-semibold"
          style={{ color: "var(--color-text-primary)" }}
        >
          Pebble
        </span>
      </div>
      <div className="flex items-center">
        <button
          onClick={() => appWindow.minimize()}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
          aria-label="Minimize"
        >
          <svg width="10" height="1" viewBox="0 0 10 1">
            <rect width="10" height="1" fill="currentColor" />
          </svg>
        </button>
        <button
          onClick={async () => {
            const maximized = await appWindow.isMaximized();
            if (maximized) {
              await appWindow.unmaximize();
            } else {
              await appWindow.maximize();
            }
          }}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-black/5"
          aria-label="Maximize"
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
          onClick={() => appWindow.close()}
          className="h-9 w-11 inline-flex items-center justify-center hover:bg-red-500 hover:text-white"
          aria-label="Close"
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <path d="M1,1 L9,9 M9,1 L1,9" stroke="currentColor" strokeWidth="1.2" />
          </svg>
        </button>
      </div>
    </div>
  );
}
