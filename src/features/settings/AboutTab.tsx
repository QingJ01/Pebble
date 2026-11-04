import { useEffect, useRef, useState } from "react";
import type { ReactNode, Ref } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { Copy, RefreshCw, X } from "lucide-react";
import iconUrl from "@/assets/app-icon.png";
import { readAppLog, type AppLogSnapshot } from "@/lib/api";

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
        error: err instanceof Error ? err.message : String(err),
      });
    }
  }

  async function loadDiagnosticLog() {
    setDiagnosticLog({ status: "loading" });
    try {
      const snapshot = await readAppLog(DIAGNOSTIC_LOG_MAX_BYTES);
      setDiagnosticLog({ status: "ready", snapshot });
    } catch (err) {
      setDiagnosticLog({
        status: "error",
        error: err instanceof Error ? err.message : String(err),
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
          <button
            onClick={handleCheckUpdate}
            disabled={update.status === "checking"}
            style={{
              padding: "8px 18px",
              fontSize: "13px",
              fontWeight: 500,
              border: "1px solid var(--color-border)",
              borderRadius: "6px",
              backgroundColor: "var(--color-bg-hover)",
              color: "var(--color-text-primary)",
              cursor: update.status === "checking" ? "wait" : "pointer",
              opacity: update.status === "checking" ? 0.6 : 1,
            }}
          >
            {update.status === "checking"
              ? t("about.checking", "Checking...")
              : t("about.checkUpdate", "Check for updates")}
          </button>

          {update.status === "latest" && (
            <span style={{ fontSize: "13px", color: "#22c55e" }}>
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
              backgroundColor: "rgba(59, 130, 246, 0.08)",
              border: "1px solid rgba(59, 130, 246, 0.2)",
              fontSize: "13px",
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
            <button
              onClick={() => openUrl(update.releaseUrl || RELEASES_URL)}
              style={{
                padding: "6px 14px",
                fontSize: "12px",
                fontWeight: 500,
                border: "none",
                borderRadius: "6px",
                backgroundColor: "var(--color-accent, #3b82f6)",
                color: "#fff",
                cursor: "pointer",
              }}
            >
              {t("about.download", "Download")}
            </button>
          </div>
        )}

        {update.status === "error" && (
          <div
            style={{
              marginTop: "12px",
              padding: "10px 14px",
              borderRadius: "6px",
              backgroundColor: "rgba(220, 53, 69, 0.1)",
              border: "1px solid rgba(220, 53, 69, 0.3)",
              fontSize: "13px",
              color: "#dc3545",
            }}
          >
            {t("about.checkFailed", "Failed to check for updates")}: {update.error}
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
                color: "var(--color-accent, #3b82f6)",
                textDecoration: "none",
                cursor: "pointer",
              }}
            >
              {link.label}
            </a>
          ))}
        </div>
      </div>

      {diagnosticLog && (
        <DiagnosticLogDialog
          state={diagnosticLog}
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
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const snapshot = state.status === "ready" ? state.snapshot : null;

  useEffect(() => {
    closeButtonRef.current?.focus();
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  function handleCopyPath() {
    if (!snapshot) {
      return;
    }
    void navigator.clipboard?.writeText(snapshot.path);
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="diagnostic-log-title"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 1200,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: "24px",
        backgroundColor: "rgba(0, 0, 0, 0.48)",
      }}
    >
      <div
        style={{
          width: "min(820px, calc(100vw - 48px))",
          maxHeight: "min(720px, calc(100vh - 48px))",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          border: "1px solid var(--color-border)",
          borderRadius: "10px",
          backgroundColor: "var(--color-bg)",
          boxShadow: "0 24px 70px rgba(0, 0, 0, 0.32)",
        }}
      >
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: "12px",
            padding: "16px 18px",
            borderBottom: "1px solid var(--color-border)",
          }}
        >
          <div style={{ minWidth: 0 }}>
            <h3
              id="diagnostic-log-title"
              style={{ margin: 0, fontSize: "15px", fontWeight: 600, color: "var(--color-text-primary)" }}
            >
              {title}
            </h3>
            {snapshot && (
              <div
                style={{
                  marginTop: "6px",
                  fontSize: "12px",
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
          <div style={{ display: "flex", alignItems: "center", gap: "8px", flexShrink: 0 }}>
            <IconButton
              label={refreshLabel}
              onClick={onRefresh}
              disabled={state.status === "loading"}
              icon={<RefreshCw size={16} />}
            />
            <IconButton
              label={copyPathLabel}
              onClick={handleCopyPath}
              disabled={!snapshot}
              icon={<Copy size={16} />}
            />
            <IconButton
              buttonRef={closeButtonRef}
              label={closeLabel}
              onClick={onClose}
              icon={<X size={16} />}
            />
          </div>
        </div>

        {state.status === "loading" && (
          <div style={{ padding: "22px", fontSize: "13px", color: "var(--color-text-secondary)" }}>
            {loadingLabel}
          </div>
        )}

        {state.status === "error" && (
          <div style={{ padding: "22px", fontSize: "13px", color: "#dc3545", whiteSpace: "pre-wrap" }}>
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
                  fontSize: "12px",
                  color: "var(--color-text-secondary)",
                  backgroundColor: "var(--color-bg-hover)",
                }}
              >
                {truncatedLabel}
              </div>
            )}
            <pre
              style={{
                margin: 0,
                minHeight: "260px",
                maxHeight: "560px",
                overflow: "auto",
                padding: "18px",
                fontSize: "12px",
                lineHeight: 1.6,
                color: "var(--color-text-primary)",
                backgroundColor: "var(--color-bg)",
                whiteSpace: "pre-wrap",
                wordBreak: "break-word",
                fontFamily: "Consolas, 'SFMono-Regular', Menlo, monospace",
              }}
            >
              {snapshot.content || emptyLabel}
            </pre>
          </>
        )}
      </div>
    </div>
  );
}

const IconButton = ({
  icon,
  label,
  onClick,
  disabled,
  buttonRef,
}: {
  icon: ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  buttonRef?: Ref<HTMLButtonElement>;
}) => (
  <button
    ref={buttonRef}
    type="button"
    onClick={onClick}
    disabled={disabled}
    aria-label={label}
    title={label}
    style={{
      width: "32px",
      height: "32px",
      display: "inline-flex",
      alignItems: "center",
      justifyContent: "center",
      border: "1px solid var(--color-border)",
      borderRadius: "6px",
      backgroundColor: "var(--color-bg-hover)",
      color: "var(--color-text-primary)",
      cursor: disabled ? "not-allowed" : "pointer",
      opacity: disabled ? 0.55 : 1,
    }}
  >
    {icon}
  </button>
);
