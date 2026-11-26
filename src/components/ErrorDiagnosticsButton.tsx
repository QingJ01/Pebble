import { useState } from "react";
import i18next from "i18next";
import Button from "./ui/Button";
import { readAppLog } from "@/lib/api";

const LOG_TAIL_BYTES = 16 * 1024;

/** Plain-text report for an issue: error, component stack and the recent app log. */
export async function buildErrorReport(error: Error, componentStack?: string | null): Promise<string> {
  const lines = [
    "Pebble error report",
    `Time: ${new Date().toISOString()}`,
    `Language: ${i18next.language ?? "unknown"}`,
    `User agent: ${typeof navigator !== "undefined" ? navigator.userAgent : "unknown"}`,
    "",
    `Error: ${error.message}`,
    error.stack ?? "",
  ];
  if (componentStack) lines.push("", "Component stack:", componentStack.trim());
  try {
    const log = await readAppLog(LOG_TAIL_BYTES);
    lines.push("", `Recent log (${log.path}${log.truncated ? ", truncated" : ""}):`, log.content);
  } catch {
    lines.push("", "Recent log: unavailable");
  }
  return lines.join("\n");
}

/** "Copy error details" for error boundaries, so users can attach diagnostics to a report. */
export default function ErrorDiagnosticsButton({ error, componentStack }: { error: Error; componentStack?: string | null }) {
  const [status, setStatus] = useState<"idle" | "copied" | "failed">("idle");

  async function copy() {
    try {
      await navigator.clipboard.writeText(await buildErrorReport(error, componentStack));
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
  }

  return (
    <>
      <Button variant="secondary" onClick={() => void copy()}>
        {i18next.t("errorBoundary.copyDiagnostics", "Copy error details")}
      </Button>
      {status !== "idle" && (
        <p role="status" className="error-state__status">
          {status === "copied"
            ? i18next.t("errorBoundary.copied", "Copied. Paste it into your issue report.")
            : i18next.t("errorBoundary.copyFailed", "Couldn't copy the error details.")}
        </p>
      )}
    </>
  );
}
