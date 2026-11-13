vi.mock("../../src/components/SenderIdentityNotice", () => ({ default: () => null }));
vi.mock("../../src/features/compose/SendControlsStatus", () => ({ default: () => null }));
vi.mock("../../src/features/compose/SendControlExitGuard", () => ({ default: () => null }));
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import Layout from "../../src/app/Layout";
import { useUIStore } from "../../src/stores/ui.store";

const mocks = vi.hoisted(() => {
  const settingsViewPromise = new Promise<{ default: () => JSX.Element }>(() => {});
  return {
    settingsViewPromise,
    invalidateQueries: vi.fn(),
    setNotificationsEnabled: vi.fn().mockResolvedValue(undefined),
    syncTitlebarTheme: vi.fn().mockResolvedValue(undefined),
  };
});

vi.mock("react-i18next", () => ({
  initReactI18next: {
    type: "3rdParty",
    init: vi.fn(),
  },
  useTranslation: () => ({
    t: (key: string, fallback?: string) => fallback ?? key,
  }),
}));

vi.mock("i18next", () => ({
  default: {
    use: vi.fn().mockReturnThis(),
    init: vi.fn().mockReturnThis(),
    changeLanguage: vi.fn(),
    t: (_key: string, fallback?: string) => fallback ?? _key,
  },
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    invalidateQueries: mocks.invalidateQueries,
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

vi.mock("../../src/lib/api", () => ({
  setNotificationsEnabled: mocks.setNotificationsEnabled,
  syncTitlebarTheme: mocks.syncTitlebarTheme,
}));

vi.mock("../../src/components/TitleBar", () => ({
  default: () => <div>Title bar</div>,
}));

vi.mock("../../src/components/Sidebar", async () => {
  const { useUIStore } = await import("../../src/stores/ui.store");
  return {
    default: function SidebarMock() {
      const activeView = useUIStore((s) => s.activeView);
      const setActiveView = useUIStore((s) => s.setActiveView);
      return (
        <button
          aria-current={activeView === "settings" ? "page" : undefined}
          onClick={() => setActiveView("settings")}
        >
          Settings
        </button>
      );
    },
  };
});

vi.mock("../../src/components/StatusBar", () => ({
  default: () => <div>Status bar</div>,
}));

vi.mock("../../src/components/ComposeFAB", () => ({
  default: () => null,
}));

vi.mock("../../src/features/command-palette/CommandPalette", () => ({
  default: () => null,
}));

vi.mock("../../src/features/command-palette/commands", () => ({
  buildCommands: vi.fn(() => []),
}));

vi.mock("../../src/components/ToastContainer", () => ({
  default: () => null,
}));

vi.mock("../../src/components/ConfirmDialog", () => ({
  default: () => null,
}));

vi.mock("../../src/features/inbox/InboxView", () => ({
  default: () => <div>Inbox panel</div>,
}));

vi.mock("../../src/features/settings/SettingsView", async () => mocks.settingsViewPromise);

vi.mock("../../src/features/kanban/KanbanView", () => ({
  default: () => <div>Kanban panel</div>,
}));

vi.mock("../../src/features/compose/ComposeView", () => ({
  default: () => <div>Compose panel</div>,
}));

vi.mock("../../src/features/search/SearchView", () => ({
  default: () => <div>Search panel</div>,
}));

vi.mock("../../src/features/snoozed/SnoozedView", () => ({
  default: () => <div>Snoozed panel</div>,
}));

vi.mock("../../src/features/starred/StarredView", () => ({
  default: () => <div>Starred panel</div>,
}));

vi.mock("../../src/app/useRealtimePreferenceSync", () => ({
  useRealtimePreferenceSync: vi.fn(),
}));

vi.mock("../../src/app/useRealtimeSyncTriggers", () => ({
  useRealtimeSyncTriggers: vi.fn(),
}));

vi.mock("../../src/app/useNotificationOpenNavigation", () => ({
  useNotificationOpenNavigation: vi.fn(),
}));

vi.mock("../../src/app/useCloseToBackground", () => ({
  useCloseToBackground: vi.fn(),
}));

vi.mock("../../src/app/useTrayI18n", () => ({
  useTrayI18n: vi.fn(),
}));

vi.mock("../../src/app/useMailtoOpen", () => ({
  useMailtoOpen: vi.fn(),
}));

vi.mock("../../src/hooks/useKeyboard", () => ({
  useKeyboard: vi.fn(),
}));

vi.mock("../../src/app/lazyViewPreload", () => ({
  createLazyViewPreloader: vi.fn(() => vi.fn().mockResolvedValue([])),
  scheduleLazyViewPreload: vi.fn(),
}));

vi.mock("../../src/stores/kanban.store", () => ({
  useKanbanStore: {
    getState: () => ({
      fetchCards: vi.fn().mockResolvedValue(undefined),
    }),
  },
}));

describe("Layout navigation", () => {
  beforeEach(() => {
    useUIStore.setState({
      activeView: "inbox",
      theme: "light",
      notificationsEnabled: true,
      networkStatus: "online",
    });
  });

  it("shows the loading fallback instead of leaving stale inbox content when a lazy view is pending", async () => {
    render(<Layout />);

    expect(await screen.findByText("Inbox panel")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByText("Loading...")).toBeTruthy();
    expect(screen.queryByText("Inbox panel")).toBeNull();
  });
});
