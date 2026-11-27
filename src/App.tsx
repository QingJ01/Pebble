import { Component, type ErrorInfo, type ReactNode, useEffect } from "react";
import i18next from "i18next";
import Layout from "./app/Layout";
import Button from "./components/ui/Button";
import ErrorDiagnosticsButton from "./components/ErrorDiagnosticsButton";
import { logStartupTiming } from "@/lib/startupTiming";

class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null; componentStack: string | null }
> {
  state: { error: Error | null; componentStack: string | null } = { error: null, componentStack: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ErrorBoundary]", error, info.componentStack);
    this.setState({ componentStack: info.componentStack ?? null });
  }

  render() {
    if (this.state.error) {
      return (
        <div className="error-state error-state--app">
          <h2 className="error-state__title">{i18next.t("errorBoundary.title", "Something went wrong")}</h2>
          <p className="error-state__description">
            {i18next.t("errorBoundary.appDescription", "Pebble ran into an unexpected error. Reload to continue; mail stored on this device is not affected.")}
          </p>
          <div className="error-state__actions">
            <Button variant="primary" onClick={() => window.location.reload()}>
              {i18next.t("errorBoundary.refresh", "Reload")}
            </Button>
            <ErrorDiagnosticsButton error={this.state.error} componentStack={this.state.componentStack} />
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

/** Time for the loading line to fill once the app is ready (index.html `.is-done`). */
export const SPLASH_LINE_FINISH_MS = 160;
/** Cross-fade from the splash into the app (index.html `#splash` transition). */
export const SPLASH_FADE_MS = 300;

/**
 * Finish the boot splash: the bottom line fills to 100%, then the splash
 * fades into the app and is removed. Safe to call when it is already gone.
 */
export function dismissSplash(doc: Document = document) {
  const splash = doc.getElementById("splash");
  if (!splash || splash.classList.contains("is-done")) return;
  logStartupTiming("splash fade started");
  const reduceMotion = typeof window.matchMedia === "function"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // Fill from where the line is now, not from zero.
  const fill = splash.querySelector<HTMLElement>(".splash-line-fill");
  if (fill) fill.style.width = getComputedStyle(fill).width;
  splash.classList.remove("is-migrating", "is-indeterminate");
  splash.classList.add("is-done");
  const finish = reduceMotion ? 0 : SPLASH_LINE_FINISH_MS;
  setTimeout(() => {
    splash.classList.add("is-leaving");
    setTimeout(() => {
      splash.remove();
      doc.getElementById("splash-style")?.remove();
      logStartupTiming("splash removed");
    }, reduceMotion ? 0 : SPLASH_FADE_MS);
  }, finish);
}

/**
 * The splash stays at least this long, so a fast start still reads as one
 * calm launch: hold (0.54s) + line fill (0.16s) + fade (0.3s) ≈ 1s.
 */
export const SPLASH_MIN_VISIBLE_MS = 540;

/** How much longer the splash should stay before handing off. */
export function splashHoldMs(now: number, splashStart: number | undefined): number {
  if (typeof splashStart !== "number") return 0;
  return Math.max(0, SPLASH_MIN_VISIBLE_MS - (now - splashStart));
}

export default function App() {
  useEffect(() => {
    logStartupTiming("react app mounted");
    // Hand off once the shell has painted, after the minimum splash time.
    let timer: ReturnType<typeof setTimeout> | undefined;
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        const start = (window as Window & { __splashStart?: number }).__splashStart;
        timer = setTimeout(() => dismissSplash(), splashHoldMs(Date.now(), start));
      });
    });
    return () => {
      cancelAnimationFrame(frame);
      if (timer) clearTimeout(timer);
    };
  }, []);

  return (
    <ErrorBoundary>
      <Layout />
    </ErrorBoundary>
  );
}
