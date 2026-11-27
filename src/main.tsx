import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "@/lib/query-client";
import { initializeProfileStorageNamespace } from "@/lib/profileStorage";
import { logStartupTiming } from "@/lib/startupTiming";
import "./styles/index.css";

async function bootstrap() {
  logStartupTiming("frontend entry loaded");
  await initializeProfileStorageNamespace();
  const { default: i18n } = await import("@/lib/i18n");
  const [{ default: App }, { showMainWindow }, { waitForStartup, splashApi }] = await Promise.all([
    import("./App"),
    import("@/lib/showMainWindow"),
    import("@/lib/startupGate"),
  ]);

  // Show the window right away: a database upgrade runs in the background
  // and the splash shows its progress until the backend is ready.
  void showMainWindow();
  try {
    await waitForStartup((stage) => {
      if (stage.stage !== "migrating") return;
      splashApi()?.progress(
        stage.total > 0 ? stage.done / stage.total : null,
        i18n.t("startup.migrating", "Upgrading your mail data…"),
        i18n.t("startup.migratingHint", "This happens once after an update. Please keep Pebble open."),
      );
    });
  } catch (error) {
    logStartupTiming("backend startup failed");
    splashApi()?.fail(
      i18n.t("startup.failed", "Pebble couldn't open its data"),
      error instanceof Error ? error.message : String(error),
    );
    return;
  }
  logStartupTiming("backend ready");

  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}

void bootstrap();
