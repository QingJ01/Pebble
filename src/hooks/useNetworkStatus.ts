import { useEffect, useRef } from "react";
import { useUIStore } from "@/stores/ui.store";
import { healthCheck } from "@/lib/api";
import { retryQueue } from "@/lib/retry-queue";

// Only poll when browser reports offline to detect recovery.
// When online, trust browser events — no unnecessary polling.
const RECOVERY_CHECK_INTERVAL_MS = 15_000;

export function useNetworkStatus() {
  const setNetworkStatus = useUIStore((s) => s.setNetworkStatus);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    const handleOnline = () => {
      setNetworkStatus("online");
      retryQueue.resume();
      stopPolling();
    };
    const handleOffline = () => {
      setNetworkStatus("offline");
      retryQueue.pause();
      startPolling();
    };

    function stopPolling() {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    }

    function startPolling() {
      stopPolling();
      intervalRef.current = setInterval(async () => {
        try {
          await healthCheck();
          setNetworkStatus("online");
          retryQueue.resume();
          stopPolling();
        } catch {
          // Still offline
        }
      }, RECOVERY_CHECK_INTERVAL_MS);
    }

    window.addEventListener("online", handleOnline);
    window.addEventListener("offline", handleOffline);

    // Set initial status from browser
    const isOnline = navigator.onLine;
    setNetworkStatus(isOnline ? "online" : "offline");
    if (!isOnline) {
      retryQueue.pause();
      startPolling();
    }

    return () => {
      window.removeEventListener("online", handleOnline);
      window.removeEventListener("offline", handleOffline);
      stopPolling();
    };
  }, [setNetworkStatus]);
}
