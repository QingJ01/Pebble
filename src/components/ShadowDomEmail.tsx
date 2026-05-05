import { useRef, useLayoutEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openMailtoUrl } from "@/app/useMailtoOpen";
import { sanitizeHtml } from "@/lib/sanitizeHtml";

interface ShadowDomEmailProps {
  html: string;
  className?: string;
}

export function ShadowDomEmail({ html, className }: ShadowDomEmailProps) {
  const hostRef = useRef<HTMLDivElement>(null);

  // The shadow body must be ready before paint; otherwise the reader can flash
  // from the fallback text into the sanitized HTML a frame later.
  useLayoutEffect(() => {
    if (!hostRef.current) return;
    const shadow = hostRef.current.shadowRoot
      || hostRef.current.attachShadow({ mode: "open" });

    const safeHtml = sanitizeHtml(html);
    shadow.innerHTML = `
      <style>
        :host {
          all: initial;
          display: block;
          font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
          font-size: 14px;
          color: var(--color-text-primary);
          background: transparent;
          word-break: break-word;
        }
        img { max-width: 100%; height: auto; }
        a { color: var(--color-accent); }
        .pebble-email-content {
          box-sizing: border-box;
          max-width: 100%;
          overflow-x: auto;
          color: inherit;
          background: transparent;
        }
        :host-context([data-theme="dark"]) .pebble-email-content {
          display: inline-block;
          max-width: 100%;
          color-scheme: light;
          color: #202124;
          background: #fff;
        }
        pre {
          white-space: pre-wrap;
          overflow-x: auto;
          scrollbar-color: var(--color-scrollbar-thumb) transparent;
          scrollbar-width: thin;
        }
        pre::-webkit-scrollbar {
          width: 10px;
          height: 10px;
        }
        pre::-webkit-scrollbar-thumb {
          border: 3px solid transparent;
          border-radius: 999px;
          background-clip: content-box;
          background-color: var(--color-scrollbar-thumb);
        }
        pre:hover::-webkit-scrollbar-thumb {
          background-color: var(--color-scrollbar-thumb-hover);
        }
        table { border-collapse: collapse; }
        .pebble-email-content > table[height="100%"],
        .pebble-email-content > div[height="100%"],
        .pebble-email-content > center[height="100%"],
        .pebble-email-content > table[style*="height:100%" i],
        .pebble-email-content > table[style*="height: 100%" i],
        .pebble-email-content > table[style*="height:100vh" i],
        .pebble-email-content > table[style*="height: 100vh" i],
        .pebble-email-content > div[style*="height:100%" i],
        .pebble-email-content > div[style*="height: 100%" i],
        .pebble-email-content > div[style*="height:100vh" i],
        .pebble-email-content > div[style*="height: 100vh" i],
        .pebble-email-content > center[style*="height:100%" i],
        .pebble-email-content > center[style*="height: 100%" i],
        .pebble-email-content > center[style*="height:100vh" i],
        .pebble-email-content > center[style*="height: 100vh" i] {
          height: auto !important;
          min-height: 0 !important;
        }
        td, th { word-break: normal; overflow-wrap: normal; }
        body, div { word-wrap: break-word; overflow-wrap: break-word; }
        .blocked-image {
          display: inline-block;
          padding: 6px 12px;
          font-size: 12px;
          color: #888;
          background: #f5f5f5;
          border: 1px dashed #ccc;
          border-radius: 4px;
          text-align: center;
          max-width: 100%;
          box-sizing: border-box;
        }
      </style>
      <div class="pebble-email-content">${safeHtml}</div>
    `;

    const handleClick = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;

      const anchor = target.closest<HTMLAnchorElement>("a[href]");
      const href = anchor?.getAttribute("href")?.trim();
      if (!href) return;

      if (/^mailto:/i.test(href)) {
        event.preventDefault();
        void openMailtoUrl(href);
        return;
      }

      if (/^https?:\/\//i.test(href)) {
        event.preventDefault();
        void invoke("open_external_url", { url: href })
          .catch((err) => console.warn("Failed to open email body link", err));
      }
    };

    shadow.addEventListener("click", handleClick);
    return () => {
      shadow.removeEventListener("click", handleClick);
    };
  }, [html]);

  return <div ref={hostRef} className={className} />;
}
