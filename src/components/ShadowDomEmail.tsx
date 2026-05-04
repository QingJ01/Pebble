import { useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openMailtoUrl } from "@/app/useMailtoOpen";
import { sanitizeHtml } from "@/lib/sanitizeHtml";

interface ShadowDomEmailProps {
  html: string;
  className?: string;
}

export function ShadowDomEmail({ html, className }: ShadowDomEmailProps) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
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
        table { max-width: 100%; border-collapse: collapse; }
        body, div { word-wrap: break-word; overflow-wrap: break-word; }
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
