import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { parseMailtoUrl } from "@/features/compose/mailto";
import i18n from "@/lib/i18n";
import { isComposeDirty, useComposeStore, type ComposePrefill } from "@/stores/compose.store";
import { useConfirmStore } from "@/stores/confirm.store";
import { useToastStore } from "@/stores/toast.store";

const OPEN_MAILTO_EVENT = "app:open-mailto";

interface OpenMailtoPayload {
  urls: string[];
}

function parseMailtoUrls(urls: string[]) {
  const parsed: ComposePrefill[] = [];
  const seen = new Set<string>();
  for (const url of urls) {
    if (seen.has(url)) continue;
    seen.add(url);
    const prefill = parseMailtoUrl(url);
    if (prefill) parsed.push(prefill);
  }
  return parsed;
}

async function confirmReplaceDirtyCompose() {
  if (!isComposeDirty()) return true;
  return useConfirmStore.getState().confirm({
    title: i18n.t("compose.mailtoReplaceDraftTitle", "Open mail link?"),
    message: i18n.t(
      "compose.mailtoReplaceDraftConfirm",
      "Opening this mail link will discard your unsaved draft. Continue?",
    ),
    confirmLabel: i18n.t("compose.leaveConfirm", "Discard"),
    cancelLabel: i18n.t("compose.leaveCancel", "Keep editing"),
    destructive: true,
  });
}

function reportSkippedMailtoUrls(count: number) {
  if (count <= 0) return;
  useToastStore.getState().addToast({
    type: "info",
    message: i18n.t("compose.mailtoSkippedLinks", {
      count,
      defaultValue: "Opened the first mail link. {{count}} additional link was skipped.",
      defaultValue_other: "Opened the first mail link. {{count}} additional links were skipped.",
    }),
  });
}

export async function openMailtoUrl(url: string) {
  return openMailtoUrls([url]);
}

async function openMailtoUrls(urls: string[]) {
  const parsed = parseMailtoUrls(urls);
  if (parsed.length === 0) return false;

  if (!(await confirmReplaceDirtyCompose())) return false;

  const [first, ...skipped] = parsed;
  useComposeStore.getState().openCompose("new", null, first);
  reportSkippedMailtoUrls(skipped.length);
  return true;
}

export function useMailtoOpen() {
  useEffect(() => {
    invoke<string[]>("take_pending_mailto_urls")
      .then((urls) => {
        void openMailtoUrls(urls);
      })
      .catch((err) => console.warn("Failed to read pending mailto URLs:", err));

    const unlisten = listen<OpenMailtoPayload>(OPEN_MAILTO_EVENT, (event) => {
      void openMailtoUrls(event.payload.urls);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);
}
