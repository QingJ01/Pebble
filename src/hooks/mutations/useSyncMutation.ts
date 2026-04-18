import { useMutation } from "@tanstack/react-query";
import { startSync } from "@/lib/api";
import { realtimePreferenceToPollInterval, useUIStore } from "@/stores/ui.store";

export function useSyncMutation() {
  const realtimeMode = useUIStore((s) => s.realtimeMode);
  const pollInterval = realtimePreferenceToPollInterval(realtimeMode);
  return useMutation({
    mutationFn: (accountId: string) => startSync(accountId, pollInterval),
    // Data refresh is driven by mail:sync-complete and mail:new events in StatusBar
  });
}
