import TitleBar from "../components/TitleBar";
import Sidebar from "../components/Sidebar";
import StatusBar from "../components/StatusBar";
import InboxView from "../features/inbox/InboxView";
import { useUIStore } from "../stores/ui.store";

export default function Layout() {
  const { activeView } = useUIStore();

  return (
    <div className="flex flex-col h-screen overflow-hidden">
      <TitleBar />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 min-w-0 overflow-auto">
          {activeView === "inbox" && <InboxView />}
          {activeView === "kanban" && (
            <div className="flex items-center justify-center h-full">
              <p style={{ color: "var(--color-text-secondary)" }}>
                Kanban — coming in Phase 3
              </p>
            </div>
          )}
          {activeView === "settings" && (
            <div className="flex items-center justify-center h-full">
              <p style={{ color: "var(--color-text-secondary)" }}>
                Settings — coming in Phase 4
              </p>
            </div>
          )}
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
