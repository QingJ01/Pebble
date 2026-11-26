import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { Copy, RefreshCw, X } from "lucide-react";
import iconUrl from "@/assets/app-icon.png";
import { readAppLog, type AppLogSnapshot } from "@/lib/api";
import { describeError, extractErrorMessage } from "@/lib/extractErrorMessage";
import Dialog from "@/components/ui/Dialog";
import Button, { buttonClassName } from "@/components/ui/Button";
import { SettingsNotice, SettingsSection } from "./SettingsLayout";
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
    getVersion().then(setAppVersion).catch(() => setAppVersion("0.2.0"));
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
      <section className="settings-section">
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-4)", marginBottom: "var(--space-4)" }}>
          {/* Hidden diagnostics trigger: 5 quick clicks on the icon open the log. */}
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
              borderRadius: "var(--radius-xl)",
              background: "transparent",
              cursor: "default",
              lineHeight: 0,
            }}
          >
            <img
              src={iconUrl}
              alt=""
              draggable={false}
              style={{ width: "56px", height: "56px", borderRadius: "var(--radius-xl)" }}
            />
          </button>
          <div>
            <div style={{ fontSize: "var(--font-lg)", fontWeight: "var(--weight-semibold)", color: "var(--color-text-primary)", lineHeight: "var(--leading-tight)" }}>
              Pebble
            </div>
            <div style={{ fontSize: "var(--font-base)", color: "var(--color-text-secondary)", marginTop: "var(--space-1)" }}>
              {t("about.version", "Version")} {appVersion || "..."}
            </div>
          </div>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)", fontSize: "var(--font-base)", color: "var(--color-text-secondary)", lineHeight: "var(--leading-relaxed)" }}>
          <p style={{ margin: 0 }}>
            {t(
              "about.description",
              "A local-first desktop email client built with Rust and React. Mail, search index, and attachments stay on your device. No telemetry. Outbound traffic happens only when you use a feature that requires it: mail sync with your provider, translation (sends the selected text to the service you configure), or WebDAV settings backup (runs against the server you provide).",
            )}
          </p>
          <p style={{ margin: 0 }}>
            {t(
              "about.features",
              "Supports Gmail, Outlook (experimental), IMAP, and POP3 accounts. Includes Kanban board, full-text search, snooze, rules engine, built-in translation, and WebDAV settings backup.",
            )}
          </p>
          <p className="settings-hint">
            {t("about.license", "Open source under AGPL-3.0 license.")}
          </p>
        </div>
      </section>

      <SettingsSection title={t("about.updates", "Updates")}>
        <div className="settings-actions">
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
          <SettingsNotice
            tone="info"
            style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: "var(--space-3)", fontSize: "var(--font-base)" }}
          >
            <span>
              {t("about.newVersion", "New version available: {{version}}", {
                version: update.latestVersion,
              })}
            </span>
            <Button variant="primary" size="sm" onClick={() => openUrl(update.releaseUrl || RELEASES_URL)}>
              {t("about.download", "Download")}
            </Button>
          </SettingsNotice>
        )}

        {update.status === "error" && (
          <div role="alert" className="dialog-error" title={update.error || undefined}>
            {t("about.checkFailed", "Failed to check for updates")}
            {update.error && (
              <div style={{ marginTop: "var(--space-1)", overflowWrap: "anywhere" }}>{update.error}</div>
            )}
          </div>
        )}
      </SettingsSection>

      <SettingsSection title={t("about.links", "Links")}>
        <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", gap: "var(--space-2)" }}>
          {[
            { label: "GitHub Releases", url: RELEASES_URL },
            { label: t("about.sourceCode", "Source Code"), url: `https://github.com/${REPO}` },
            { label: t("about.reportIssue", "Report an Issue"), url: `https://github.com/${REPO}/issues` },
          ].map((link) => (
            <a
              key={link.url}
              href={link.url}
              className={buttonClassName("link", "sm")}
              onClick={(e) => { e.preventDefault(); openUrl(link.url); }}
            >
              {link.label}
            </a>
          ))}
        </div>
      </SettingsSection>

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
      <div className="dialog-panel__header">
        <div style={{ flex: 1, minWidth: 0 }}>
          <h2 id={titleId} className="dialog-title">
            {title}
          </h2>
          {snapshot && (
            <div
              title={snapshot.path}
              style={{
                marginTop: "var(--space-1)",
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
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-1)", flexShrink: 0 }}>
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
        <div role="status" style={{ padding: "var(--space-5)", fontSize: "var(--font-base)", color: "var(--color-text-secondary)" }}>
          {loadingLabel}
        </div>
      )}

      {state.status === "error" && (
        <div role="alert" className="dialog-error" style={{ margin: "var(--space-5)" }}>
          {state.error}
        </div>
      )}

      {snapshot && (
        <>
          {snapshot.truncated && (
            <div
              style={{
                padding: "var(--space-2) var(--space-5)",
                borderBottom: "1px solid var(--color-divider)",
                fontSize: "var(--font-sm)",
                color: "var(--color-text-secondary)",
                backgroundColor: "var(--color-surface-sunken)",
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
              padding: "var(--space-5)",
              fontSize: "var(--font-sm)",
              lineHeight: "var(--leading-relaxed)",
              color: "var(--color-text-primary)",
              backgroundColor: "var(--color-canvas)",
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
