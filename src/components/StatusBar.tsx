import { useUIStore } from "../stores/ui.store";

export default function StatusBar() {
  const { syncStatus } = useUIStore();
  const statusText = {
    idle: "Ready",
    syncing: "Syncing...",
    error: "Sync error",
  }[syncStatus];

  return (
    <footer
      className="flex items-center px-3 h-6 text-xs border-t"
      style={{
        backgroundColor: "var(--color-statusbar-bg)",
        borderColor: "var(--color-border)",
        color: "var(--color-text-secondary)",
      }}
    >
      <span>{statusText}</span>
    </footer>
  );
}
