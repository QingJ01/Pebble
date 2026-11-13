import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import ConfirmDialog from "@/components/ConfirmDialog";
import { resolveSendControlExit, sendControlExitPending } from "@/lib/send-controls-api";

/** Mount once at the app root, outside lazy views. Native code pauses new claims. */
export default function SendControlExitGuard() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("send-controls-exit-requested", () => setOpen(true))
      .then(async (fn) => {
        if (disposed) { fn(); return; }
        unlisten = fn;
        if (await sendControlExitPending() && !disposed) setOpen(true);
      }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  async function resolve(confirm: boolean) {
    setBusy(true); setError(false);
    try { await resolveSendControlExit(confirm); setOpen(false); }
    catch { setError(true); }
    finally { setBusy(false); }
  }
  if (!open) return null;
  return <>
    {error && <p role="alert">{t("sendControls.error")}</p>}
    <ConfirmDialog title={t("sendControls.exitTitle")} message={t("sendControls.exitMessage")}
      confirmLabel={t("sendControls.exitConfirm")} cancelLabel={t("sendControls.exitCancel")}
      busy={busy} destructive onConfirm={() => void resolve(true)} onCancel={() => void resolve(false)} />
  </>;
}
