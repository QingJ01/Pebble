import TitleBar from "../components/TitleBar";
import SenderIdentityNotice from "../components/SenderIdentityNotice";
import SendControlsStatus from "../features/compose/SendControlsStatus";
import SendControlExitGuard from "../features/compose/SendControlExitGuard";
import Sidebar from "../components/Sidebar";
import StatusBar from "../components/StatusBar";
import ComposeFAB from "../components/ComposeFAB";
import InboxView from "../features/inbox/InboxView";
import CommandPalette from "../features/command-palette/CommandPalette";
import ToastContainer from "../components/ToastContainer";
import ConfirmDialog from "../components/ConfirmDialog";
import { useConfirmStore } from "../stores/confirm.store";
import { useComposeStore } from "../stores/compose.store";
import { useUIStore, applyThemeToDom, resolveTheme } from "../stores/ui.store";
import { useCommandStore } from "../stores/command.store";
import { useKanbanCardsQuery } from "@/hooks/queries/useKanbanCardsQuery";
import { useKeyboard } from "../hooks/useKeyboard";
import { useNetworkStatus } from "../hooks/useNetworkStatus";
import { buildCommands } from "../features/command-palette/commands";
import { useEffect, lazy, Suspense, Component, type ReactNode, type ErrorInfo } from "react";
import { createLazyViewPreloader, scheduleLazyViewPreload } from "./lazyViewPreload";
import { useRealtimePreferenceSync } from "./useRealtimePreferenceSync";
import { useRealtimeSyncTriggers } from "./useRealtimeSyncTriggers";
import { useNotificationOpenNavigation } from "./useNotificationOpenNavigation";
import { useCloseToBackground } from "./useCloseToBackground";
import { useTrayI18n } from "./useTrayI18n";
import { useMailtoOpen } from "./useMailtoOpen";
import AppBackground from "./AppBackground";
import Button from "../components/ui/Button";
import ErrorDiagnosticsButton from "../components/ErrorDiagnosticsButton";
import Spinner from "../components/Spinner";
import { LOADING_INDICATOR_DELAY_MS } from "../hooks/useDelayedVisible";
import "./shell.css";

const loadSettingsView = () => import("../features/settings/SettingsView");
const loadComposeView = () => import("../features/compose/ComposeView");
const loadKanbanView = () => import("../features/kanban/KanbanView");
const loadContactsView = () => import("../features/contacts/ContactsView");
const loadSearchView = () => import("../features/search/SearchView");
const loadSnoozedView = () => import("../features/snoozed/SnoozedView");
const loadStarredView = () => import("../features/starred/StarredView");
const preloadLazyViews = createLazyViewPreloader([
  loadSettingsView,
  loadComposeView,
  loadKanbanView,
  loadContactsView,
  loadSearchView,
  loadSnoozedView,
  loadStarredView,
]);

const SettingsView = lazy(loadSettingsView);
const ComposeView = lazy(loadComposeView);
const KanbanView = lazy(loadKanbanView);
const ContactsView = lazy(loadContactsView);
const SearchView = lazy(loadSearchView);
const SnoozedView = lazy(loadSnoozedView);
const StarredView = lazy(loadStarredView);
import { useTranslation } from "react-i18next";
import i18next from "i18next";
import { WifiOff } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { setNotificationsEnabled as setBackendNotificationsEnabled, syncTitlebarTheme } from "@/lib/api";

export default function Layout() {
  const activeView = useUIStore((s) => s.activeView);
  const displayedView = activeView;
  const composeKey = useComposeStore((s) => s.composeKey);
  const setActiveView = useUIStore((s) => s.setActiveView);
  const theme = useUIStore((s) => s.theme);
  const backgroundImage = useUIStore((s) => s.backgroundImage);
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  useKeyboard();

  // Load kanban cards at startup so MessageItem can show kanban indicators
  useKanbanCardsQuery();

  useEffect(() => scheduleLazyViewPreload(preloadLazyViews), []);

  useNetworkStatus();
  useRealtimePreferenceSync();
  useRealtimeSyncTriggers();
  useNotificationOpenNavigation();
  useCloseToBackground();
  useTrayI18n();
  useMailtoOpen();

  // Re-register commands when language changes
  useEffect(() => {
    useCommandStore.getState().registerCommands(buildCommands(t));
  }, [t]);

  // Keep the Rust notification gate aligned with the single frontend preference source.
  useEffect(() => {
    setBackendNotificationsEnabled(notificationsEnabled)
      .catch((err) => console.warn("Failed to sync notification preference to backend", err));
  }, [notificationsEnabled]);

  // Global listener: refresh data when snoozed messages are restored
  useEffect(() => {
    const unlisten = listen<{ message_id: string; return_to?: string }>("mail:unsnoozed", (event) => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["snoozed"] });

      const { return_to } = event.payload;
      if (return_to) {
        if (return_to.startsWith("kanban")) {
          setActiveView("kanban");
        } else if (return_to === "inbox" || return_to === "starred" || return_to === "search") {
          setActiveView(return_to as "inbox" | "starred" | "search");
        }
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [queryClient, setActiveView]);

  useEffect(() => {
    applyThemeToDom(theme);
    syncTitlebarTheme(resolveTheme(theme)).catch(() => {});
    if (theme === "system") {
      const mql = window.matchMedia("(prefers-color-scheme: dark)");
      const listener = () => {
        applyThemeToDom("system");
        syncTitlebarTheme(resolveTheme("system")).catch(() => {});
      };
      mql.addEventListener("change", listener);
      return () => mql.removeEventListener("change", listener);
    }
  }, [theme]);

  return (
    <div
      className={`app-shell flex flex-col h-screen overflow-hidden${backgroundImage ? " app-shell--with-background" : ""}`}
    >
      <AppBackground image={backgroundImage} />
      <TitleBar />
      <SenderIdentityNotice />
      <SendControlsStatus />
      <OfflineBanner />
      <div className="flex flex-1 min-h-0 app-shell-content">
        <Sidebar />
        <main className="flex-1 min-w-0 overflow-auto scroll-region app-main-scroll" style={{ position: "relative" }}>
          <ViewErrorBoundary key={displayedView}>
            <Suspense fallback={<ViewLoadingFallback />}>
              {displayedView === "inbox" && <InboxView />}
              {displayedView === "kanban" && <KanbanView />}
              {displayedView === "contacts" && <ContactsView />}
              {displayedView === "settings" && <SettingsView />}
              {displayedView === "search" && <SearchView />}
              {displayedView === "snoozed" && <SnoozedView />}
              {displayedView === "starred" && <StarredView />}
              {displayedView === "compose" && <ComposeView key={composeKey} />}
            </Suspense>
          </ViewErrorBoundary>
        </main>
      </div>
      <ComposeFAB />
      <StatusBar />
      <CommandPalette />
      <ToastContainer />
      <SendControlExitGuard />
      <GlobalConfirmDialog />
    </div>
  );
}

function ViewLoadingFallback() {
  // Lazy views are usually preloaded; only show a spinner if loading is slow.
  return (
    <div className="view-loading">
      <Spinner delayMs={LOADING_INDICATOR_DELAY_MS} label={i18next.t("common.loading", "Loading...")} />
    </div>
  );
}

class ViewErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null; componentStack?: string | null }
> {
  state: { error: Error | null; componentStack?: string | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ViewError]", error, info.componentStack);
    this.setState({ componentStack: info.componentStack ?? null });
  }

  render() {
    if (this.state.error) {
      return (
        <div className="error-state">
          <p className="error-state__title">{i18next.t("errorBoundary.title", "Something went wrong")}</p>
          <p className="error-state__description">
            {i18next.t("errorBoundary.description", "This view ran into an unexpected error. Try again, or reload Pebble if it keeps happening.")}
          </p>
          {this.state.error && import.meta.env.DEV && (
            <pre className="error-state__detail">
              {this.state.error.message}
              {"\n"}
              {this.state.error.stack}
            </pre>
          )}
          <div className="error-state__actions">
            <Button variant="primary" onClick={() => this.setState({ error: null, componentStack: null })}>
              {i18next.t("errorBoundary.retry", "Retry")}
            </Button>
            <ErrorDiagnosticsButton error={this.state.error} componentStack={this.state.componentStack} />
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

function GlobalConfirmDialog() {
  const { isOpen, title, message, destructive, confirmLabel, cancelLabel, handleConfirm, handleCancel } = useConfirmStore();
  // Stays mounted so closing plays the exit animation (the store keeps the copy).
  return (
    <ConfirmDialog
      open={isOpen}
      title={title}
      message={message}
      destructive={destructive}
      confirmLabel={confirmLabel}
      cancelLabel={cancelLabel}
      onConfirm={handleConfirm}
      onCancel={handleCancel}
    />
  );
}

function OfflineBanner() {
  const { t } = useTranslation();
  const networkStatus = useUIStore((s) => s.networkStatus);
  if (networkStatus === "online") return null;
  return (
    <div className="offline-banner" role="status" aria-live="polite">
      <WifiOff size={14} aria-hidden="true" />
      <span>
        <strong className="offline-banner__title">{t("status.offline", "Offline")}</strong>
        {" — "}
        {t("status.offlineHint", "Mail sync is paused until you're back online")}
      </span>
    </div>
  );
}
