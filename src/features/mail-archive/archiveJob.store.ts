import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import type { ArchiveProgress, ArchiveReport } from "@/lib/mail-archive-api";

export type ArchiveReportKind = "preview" | "import" | "export";

/**
 * Local-archive job state lives outside MailArchiveView so an import/export
 * keeps running (and reporting progress) when the user switches settings tabs
 * or views; returning to the page shows the live job or its final report.
 */
interface ArchiveJobState {
  busy: boolean;
  jobId: string | null;
  progress: ArchiveProgress | null;
  error: string;
  report: ArchiveReport | null;
  reportKind: ArchiveReportKind;
  preview: ArchiveReport | null;
  /** Whether MailArchiveView is currently mounted (drives completion toasts). */
  viewMounted: boolean;
  set: (patch: Partial<Omit<ArchiveJobState, "set">>) => void;
}

const INITIAL_STATE = {
  busy: false,
  jobId: null,
  progress: null,
  error: "",
  report: null,
  reportKind: "import",
  preview: null,
  viewMounted: false,
} satisfies Omit<ArchiveJobState, "set">;

export const useArchiveJobStore = create<ArchiveJobState>((set) => ({
  ...INITIAL_STATE,
  set: (patch) => set(patch),
}));

/** Test helper: forget any finished or running job state. */
export function resetArchiveJobStore() {
  useArchiveJobStore.setState(INITIAL_STATE);
}

let progressListener: Promise<() => void> | null = null;

/** Subscribe (once, app-wide) to job progress events. */
export function ensureArchiveProgressListener() {
  if (progressListener) return;
  progressListener = listen<ArchiveProgress>("mail-archive:progress", ({ payload }) => {
    const state = useArchiveJobStore.getState();
    if (payload.job_id === state.jobId) state.set({ progress: payload });
  });
  progressListener.catch(() => { progressListener = null; });
}
