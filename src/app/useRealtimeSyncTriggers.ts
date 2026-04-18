import { useEffect } from "react";
import { triggerSync } from "@/lib/api";
import { useMailStore } from "@/stores/mail.store";
import { useUIStore } from "@/stores/ui.store";

export function useRealtimeSyncTriggers() {
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const networkStatus = useUIStore((s) => s.networkStatus);

  useEffect(() => {
    if (!activeAccountId) return;

    const onFocus = () => {
      triggerSync(activeAccountId, "window_focus").catch(() => {});
    };
    const onBlur = () => {
      triggerSync(activeAccountId, "window_blur").catch(() => {});
    };

    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
    };
  }, [activeAccountId]);

  useEffect(() => {
    if (!activeAccountId || networkStatus !== "online") return;
    triggerSync(activeAccountId, "network_online").catch(() => {});
  }, [activeAccountId, networkStatus]);
}
