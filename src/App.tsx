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

/** Splash fade duration; matches the transition in index.html. */
export const SPLASH_FADE_MS = 200;

/** Fade out and remove the boot splash. Safe to call when it is already gone. */
export function dismissSplash(doc: Document = document) {
  const splash = doc.getElementById("splash");
  if (!splash || splash.classList.contains("fade-out")) return;
  logStartupTiming("splash fade started");
  splash.classList.add("fade-out");
  const reduceMotion = typeof window.matchMedia === "function"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  setTimeout(() => {
    splash.remove();
    doc.getElementById("splash-style")?.remove();
    logStartupTiming("splash removed");
  }, reduceMotion ? 0 : SPLASH_FADE_MS);
}

export default function App() {
  useEffect(() => {
    logStartupTiming("react app mounted");
    // Fade as soon as the shell has painted once — no artificial minimum.
    let frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => dismissSplash());
    });
    return () => cancelAnimationFrame(frame);
  }, []);

  return (
    <ErrorBoundary>
      <Layout />
    </ErrorBoundary>
  );
}
