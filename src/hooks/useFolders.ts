import { useEffect } from "react";
import { useMailStore } from "@/stores/mail.store";

export function useFolders() {
  const folders = useMailStore((s) => s.folders);
  const loading = useMailStore((s) => s.loadingFolders);
  const activeFolderId = useMailStore((s) => s.activeFolderId);
  const setActiveFolder = useMailStore((s) => s.setActiveFolder);
  const activeAccountId = useMailStore((s) => s.activeAccountId);
  const fetchFolders = useMailStore((s) => s.fetchFolders);

  useEffect(() => {
    if (activeAccountId) {
      fetchFolders(activeAccountId);
    }
  }, [activeAccountId, fetchFolders]);

  return { folders, loading, activeFolderId, setActiveFolder };
}
