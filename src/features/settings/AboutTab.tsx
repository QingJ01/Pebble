import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { Copy, RefreshCw, X } from "lucide-react";
import iconUrl from "@/assets/app-icon.png";
import { readAppLog, type AppLogSnapshot } from "@/lib/api";
import { describeError, extractErrorMessage } from "@/lib/extractErrorMessage";
import Dialog from "@/components/ui/Dialog";
import Button from "@/components/ui/Button";
import IconButton from "@/components/ui/IconButton";

const REPO = "QingJ01/Pebble";
const RELEASES_URL = `https://github.com/${REPO}/releases`;

function openUrl(url: string) {
  invoke("open_external_url", { url }).catch((err) => console.warn("Failed to open external URL", err));
}

interface UpdateState {
  status: "idle" | "checking" | "latest" | "available" | "error";
  latestVersion?: string;
  releaseUrl?: string;
  error?: string;
}

type DiagnosticLogState =
  | { status: "loading" }
  | { status: "ready"; snapshot: AppLogSnapshot }
  | { status: "error"; error: string };

const DIAGNOSTIC_CLICK_TARGET = 5;
const DIAGNOSTIC_CLICK_WINDOW_MS = 1500;
const DIAGNOSTIC_LOG_MAX_BYTES = 64 * 1024;

export default function AboutTab() {
  const { t } = useTranslation();
  const [appVersion, setAppVersion] = useState<string>("");
  const [update, setUpdate] = useState<UpdateState>({ status: "idle" });
  const [diagnosticLog, setDiagnosticLog] = useState<DiagnosticLogState | null>(null);
  // Keep the last state while the dialog plays its exit animation.
  const diagnosticDialog = useRef<DiagnosticLogState | null>(null);
  if (diagnosticLog) diagnosticDialog.current = diagnosticLog;
  const diagnosticClickCount = useRef(0);
  const diagnosticClickTimer = useRef<number | null>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => setAppVersion("0.1.6"));
  }, []);

  useEffect(() => () => {
    if (diagnosticClickTimer.current !== null) {
      window.clearTimeout(diagnosticClickTimer.current);
    }
  }, []);

  async function handleCheckUpdate() {
    setUpdate({ status: "checking" });
    try {
      const info = await invoke<{
        latest_version: string;
        release_url: string;
        is_newer: boolean;
      }>("check_for_update", { currentVersion: appVersion });
      if (info.is_newer) {
        setUpdate({
          status: "available",
          latestVersion: info.latest_version,
          releaseUrl: info.release_url,
        });
      } else {
        setUpdate({ status: "latest", latestVersion: info.latest_version });
      }
    } catch (err) {
      setUpdate({
        status: "error",
        error: extractErrorMessage(err),
      });
    }
  }

  async function loadDiagnosticLog() {
    setDiagnosticLog({ status: "loading" });
    try {
      const snapshot = await readAppLog(DIAGNOSTIC_LOG_MAX_BYTES);
      setDiagnosticLog({ status: "ready", snapshot });
    } catch (err) {
      const described = describeError(err);
      setDiagnosticLog({
        status: "error",
        error: described.detail && described.detail !== described.title
          ? `${described.title}\n${described.detail}`
          : described.title,
      });
    }
  }

  function handleDiagnosticIconClick() {
    diagnosticClickCount.current += 1;

    if (diagnosticClickTimer.current !== null) {
      window.clearTimeout(diagnosticClickTimer.current);
    }

    if (diagnosticClickCount.current >= DIAGNOSTIC_CLICK_TARGET) {
      diagnosticClickCount.current = 0;
      void loadDiagnosticLog();
      return;
    }

    diagnosticClickTimer.current = window.setTimeout(() => {
      diagnosticClickCount.current = 0;
      diagnosticClickTimer.current = null;
    }, DIAGNOSTIC_CLICK_WINDOW_MS);
  }

  return (
    <div>
      <h2
        style={{
          fontSize: "18px",
          fontWeight: 600,
          color: "var(--color-text-primary)",
          marginTop: 0,
          marginBottom: "24px",
        }}
      >
        {t("about.title", "About")}
      </h2>

      {/* App info */}
      <div style={{ marginBottom: "24px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "14px", marginBottom: "16px" }}>
          <button
            type="button"
            onClick={handleDiagnosticIconClick}
            aria-label={t("about.openDiagnosticLog", "Open diagnostic log")}
            title="Pebble"
            style={{
              width: "56px",
              height: "56px",
              padding: 0,
              border: "none",
              borderRadius: "12px",
              background: "transparent",
              cursor: "default",
              lineHeight: 0,
            }}
          >
            <img
              src={iconUrl}
              alt=""
              draggable={false}
              style={{ width: "56px", height: "56px", borderRadius: "12px" }}
            />
          </button>
          <div>
            <div style={{ fontSize: "17px", fontWeight: 600, color: "var(--color-text-primary)" }}>
              Pebble
            </div>
            <div style={{ fontSize: "13px", color: "var(--color-text-secondary)", marginTop: "2px" }}>
              {t("about.version", "Version")} {appVersion || "..."}
            </div>
          </div>
        </div>

        <p style={{ fontSize: "13px", color: "var(--color-text-secondary)", lineHeight: 1.7, margin: "0 0 8px" }}>
          {t(
            "about.description",
            "A local-first desktop email client built with Rust and React. Mail, search index, and attachments stay on your device. No telemetry. Outbound traffic happens only when you use a feature that requires it: mail sync with your provider, translation (sends the selected text to the service you configure), or WebDAV settings backup (runs against the server you provide).",
          )}
        </p>
        <p style={{ fontSize: "13px", color: "var(--color-text-secondary)", lineHeight: 1.7, margin: "0 0 8px" }}>
          {t(
            "about.features",
            "Supports Gmail, Outlook (experimental), IMAP, and POP3 accounts. Includes Kanban board, full-text search, snooze, rules engine, built-in translation, and WebDAV settings backup.",
          )}
        </p>
        <p style={{ fontSize: "12px", color: "var(--color-text-tertiary, var(--color-text-secondary))", lineHeight: 1.5, margin: 0 }}>
          {t("about.license", "Open source under AGPL-3.0 license.")}
        </p>
      </div>

      {/* Check for updates */}
      <div style={{ marginBottom: "24px" }}>
        <h3
          style={{
            fontSize: "14px",
            fontWeight: 600,
            color: "var(--color-text-primary)",
            marginTop: 0,
            marginBottom: "12px",
          }}
        >
          {t("about.updates", "Updates")}
        </h3>

        <div style={{ display: "flex", alignItems: "center", gap: "12px" }}>
          <Button
            variant="secondary"
            onClick={handleCheckUpdate}
            loading={update.status === "checking"}
          >
            {update.status === "checking"
              ? t("about.checking", "Checking...")
              : t("about.checkUpdate", "Check for updates")}
          </Button>

          {update.status === "latest" && (
            <span role="status" style={{ fontSize: "var(--font-base)", color: "var(--color-success)" }}>
              {t("about.upToDate", "You're on the latest version")}
            </span>
          )}
        </div>

        {update.status === "available" && (
          <div
            style={{
              marginTop: "12px",
              padding: "12px 14px",
              borderRadius: "6px",
              gap: "12px",
              backgroundColor: "var(--color-info-bg)",
              border: "1px solid var(--color-border)",
              fontSize: "var(--font-base)",
              color: "var(--color-text-primary)",
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
            }}
          >
            <span>
              {t("about.newVersion", "New version available: {{version}}", {
                version: update.latestVersion,
              })}
            </span>
            <Button variant="primary" size="sm" onClick={() => openUrl(update.releaseUrl || RELEASES_URL)}>
              {t("about.download", "Download")}
            </Button>
          </div>
        )}

        {update.status === "error" && (
          <div role="alert" className="dialog-error" style={{ marginTop: "12px" }} title={update.error || undefined}>
            {t("about.checkFailed", "Failed to check for updates")}
            {update.error && (
              <div style={{ marginTop: "2px", opacity: 0.85, overflowWrap: "anywhere" }}>{update.error}</div>
            )}
          </div>
        )}
      </div>

      {/* Links */}
      <div>
        <h3
          style={{
            fontSize: "14px",
            fontWeight: 600,
            color: "var(--color-text-primary)",
            marginTop: 0,
            marginBottom: "12px",
          }}
        >
          {t("about.links", "Links")}
        </h3>
        <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
          {[
            { label: "GitHub Releases", url: RELEASES_URL },
            { label: t("about.sourceCode", "Source Code"), url: `https://github.com/${REPO}` },
            { label: t("about.reportIssue", "Report an Issue"), url: `https://github.com/${REPO}/issues` },
          ].map((link) => (
            <a
              key={link.url}
              href={link.url}
              onClick={(e) => { e.preventDefault(); openUrl(link.url); }}
              style={{
                fontSize: "13px",
                color: "var(--color-accent-text)",
                textDecoration: "none",
                cursor: "pointer",
              }}
            >
              {link.label}
            </a>
          ))}
        </div>
      </div>

      {diagnosticDialog.current && (
        <DiagnosticLogDialog
          open={diagnosticLog !== null}
          state={diagnosticDialog.current}
          title={t("about.diagnosticLog", "Diagnostic log")}
          loadingLabel={t("common.loading", "Loading...")}
          emptyLabel={t("about.noDiagnosticLog", "No log entries yet.")}
          truncatedLabel={t("about.diagnosticLogTruncated", "Showing latest 64 KB")}
          refreshLabel={t("common.retry", "Retry")}
          copyPathLabel={t("about.copyLogPath", "Copy path")}
          closeLabel={t("common.close", "Close")}
          onRefresh={() => void loadDiagnosticLog()}
          onClose={() => setDiagnosticLog(null)}
        />
      )}
    </div>
  );
}

function DiagnosticLogDialog({
  open,
  state,
  title,
  loadingLabel,
  emptyLabel,
  truncatedLabel,
  refreshLabel,
  copyPathLabel,
  closeLabel,
  onRefresh,
  onClose,
}: {
  open: boolean;
  state: DiagnosticLogState;
  title: string;
  loadingLabel: string;
  emptyLabel: string;
  truncatedLabel: string;
  refreshLabel: string;
  copyPathLabel: string;
  closeLabel: string;
  onRefresh: () => void;
  onClose: () => void;
}) {
  const titleId = useId();
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const snapshot = state.status === "ready" ? state.snapshot : null;

  function handleCopyPath() {
    if (!snapshot) {
      return;
    }
    void navigator.clipboard?.writeText(snapshot.path);
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      labelledBy={titleId}
      closeOnBackdrop={false}
      initialFocus={closeButtonRef}
      panelClassName="dialog-panel dialog-panel--flush"
      style={{
        width: "min(820px, 100%)",
        maxHeight: "min(720px, calc(100vh - 48px))",
        overflow: "hidden",
      }}
    >
      <div
        className="dialog-header"
        style={{
          gap: "12px",
          padding: "16px 18px",
          borderBottom: "1px solid var(--color-border)",
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <h2 id={titleId} className="dialog-title">
            {title}
          </h2>
          {snapshot && (
            <div
              title={snapshot.path}
              style={{
                marginTop: "4px",
                fontSize: "var(--font-sm)",
                color: "var(--color-text-secondary)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {snapshot.path}
            </div>
          )}
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "4px", flexShrink: 0 }}>
          <IconButton
            size="md"
            label={refreshLabel}
            onClick={onRefresh}
            loading={state.status === "loading"}
            icon={<RefreshCw size={16} />}
          />
          <IconButton
            size="md"
            label={copyPathLabel}
            onClick={handleCopyPath}
            disabled={!snapshot}
            icon={<Copy size={16} />}
          />
          <IconButton
            ref={closeButtonRef}
            size="md"
            label={closeLabel}
            onClick={onClose}
            icon={<X size={16} />}
          />
        </div>
      </div>

      {state.status === "loading" && (
        <div role="status" style={{ padding: "22px", fontSize: "var(--font-base)", color: "var(--color-text-secondary)" }}>
          {loadingLabel}
        </div>
      )}

      {state.status === "error" && (
        <div role="alert" className="dialog-error" style={{ margin: "18px" }}>
          {state.error}
        </div>
      )}

      {snapshot && (
        <>
          {snapshot.truncated && (
            <div
              style={{
                padding: "8px 18px",
                borderBottom: "1px solid var(--color-border)",
                fontSize: "var(--font-sm)",
                color: "var(--color-text-secondary)",
                backgroundColor: "var(--color-bg-hover)",
              }}
            >
              {truncatedLabel}
            </div>
          )}
          <pre
            tabIndex={0}
            style={{
              margin: 0,
              minHeight: "260px",
              flex: 1,
              overflow: "auto",
              padding: "18px",
              fontSize: "var(--font-sm)",
              lineHeight: 1.6,
              color: "var(--color-text-primary)",
              backgroundColor: "var(--color-bg)",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              fontFamily: "var(--font-mono)",
              userSelect: "text",
            }}
          >
            {snapshot.content || emptyLabel}
          </pre>
        </>
      )}
    </Dialog>
  );
}
