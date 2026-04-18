import { useEffect } from "react";
import { setRealtimePreference } from "@/lib/api";
import { useUIStore } from "@/stores/ui.store";

export function useRealtimePreferenceSync() {
  const realtimeMode = useUIStore((state) => state.realtimeMode);

  useEffect(() => {
    setRealtimePreference(realtimeMode).catch(() => {});
  }, [realtimeMode]);
}
