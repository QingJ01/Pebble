import TitleBar from "../components/TitleBar";
import Sidebar from "../components/Sidebar";
import StatusBar from "../components/StatusBar";
import ComposeFAB from "../components/ComposeFAB";
import InboxView from "../features/inbox/InboxView";
import SettingsView from "../features/settings/SettingsView";
import CommandPalette from "../features/command-palette/CommandPalette";
import ComposeView from "../features/compose/ComposeView";
import KanbanView from "../features/kanban/KanbanView";
import SearchView from "../features/search/SearchView";
import SnoozedView from "../features/snoozed/SnoozedView";
import StarredView from "../features/starred/StarredView";
import ToastContainer from "../components/ToastContainer";
import { useUIStore, applyThemeToDom } from "../stores/ui.store";
import { useCommandStore } from "../stores/command.store";
import { useKanbanStore } from "../stores/kanban.store";
import { useKeyboard } from "../hooks/useKeyboard";
import { useNetworkStatus } from "../hooks/useNetworkStatus";
import { buildCommands } from "../features/command-palette/commands";
import { useEffect, Component, type ReactNode, type ErrorInfo } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";

export default function Layout() {
  const activeView = useUIStore((s) => s.activeView);
  const setActiveView = useUIStore((s) => s.setActiveView);
  const theme = useUIStore((s) => s.theme);
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  useKeyboard();

  // Load kanban cards at startup so MessageItem can show kanban indicators
  useEffect(() => {
    useKanbanStore.getState().fetchCards();
  }, []);
  useNetworkStatus();

  // Re-register commands when language changes
  useEffect(() => {
    useCommandStore.getState().registerCommands(buildCommands(t));
    // Sync notification preference from localStorage to backend on startup
    const enabled = localStorage.getItem("pebble-notifications-enabled") === "true";
    invoke("set_notifications_enabled", { enabled }).catch(() => {});
  }, [t]);

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
    if (theme === "system") {
      const mql = window.matchMedia("(prefers-color-scheme: dark)");
      const listener = () => applyThemeToDom("system");
      mql.addEventListener("change", listener);
      return () => mql.removeEventListener("change", listener);
    }
  }, [theme]);

  return (
    <div className="flex flex-col h-screen overflow-hidden">
      <TitleBar />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 min-w-0 overflow-auto" style={{ position: "relative" }}>
          <ViewErrorBoundary key={activeView}>
            {activeView === "inbox" && <InboxView />}
            {activeView === "kanban" && <KanbanView />}
            {activeView === "settings" && <SettingsView />}
            {activeView === "search" && <SearchView />}
            {activeView === "snoozed" && <SnoozedView />}
            {activeView === "starred" && <StarredView />}
            {activeView === "compose" && <ComposeView />}
          </ViewErrorBoundary>
        </main>
      </div>
      <ComposeFAB />
      <StatusBar />
      <CommandPalette />
      <ToastContainer />
    </div>
  );
}

class ViewErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ViewError]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{
          display: "flex", flexDirection: "column", alignItems: "center",
          justifyContent: "center", height: "100%", gap: 12, padding: 24,
          color: "var(--color-text-secondary)",
        }}>
          <p style={{ fontSize: 14, margin: 0 }}>Something went wrong</p>
          <p style={{ fontSize: 12, margin: 0, color: "var(--color-text-secondary)" }}>
            Please try again or refresh the application.
          </p>
          <button
            onClick={() => this.setState({ error: null })}
            style={{
              padding: "6px 16px", cursor: "pointer",
              backgroundColor: "var(--color-accent)", color: "#fff",
              border: "none", borderRadius: 6, fontSize: 13,
            }}
          >
            Retry
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
