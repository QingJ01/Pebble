import { useRef, useLayoutEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openMailtoUrl } from "@/app/useMailtoOpen";
import { sanitizeHtml } from "@/lib/sanitizeHtml";

interface ShadowDomEmailProps {
  html: string;
  className?: string;
}

function scrollToEmailFragment(root: ShadowRoot, fragment: string) {
  let id = fragment;
  try {
    id = decodeURIComponent(fragment);
  } catch {
    // Keep the raw fragment.
  }
  const target = Array.from(root.querySelectorAll<HTMLElement>("[id], a[name]")).find(
    (element) => element.id === id || element.getAttribute("name") === id,
  );
  target?.scrollIntoView?.({ block: "start" });
}

const PLAIN_TEXT_RE = /^\s*<pre class="pebble-plain-text-email">[\s\S]*<\/pre>\s*$/;

export function ShadowDomEmail({ html, className }: ShadowDomEmailProps) {
  const hostRef = useRef<HTMLDivElement>(null);

  // The shadow body must be ready before paint; otherwise the reader can flash
  // from the fallback text into the sanitized HTML a frame later.
  useLayoutEffect(() => {
    if (!hostRef.current) return;
    const shadow = hostRef.current.shadowRoot
      || hostRef.current.attachShadow({ mode: "open" });

    const safeHtml = sanitizeHtml(html);
    // Plain-text mail arrives wrapped as <pre class="pebble-plain-text-email">;
    // it has no authored colors, so it follows the app theme instead of
    // sitting on the light HTML-mail card in dark mode.
    const isPlainText = PLAIN_TEXT_RE.test(safeHtml);
    shadow.innerHTML = `
      <style>
        :host {
          all: initial;
          display: block;
          /* Confine email CSS: position:fixed descendants resolve against this
             box and are clipped to it instead of overlaying the app UI. */
          position: relative;
          contain: layout paint;
          isolation: isolate;
          transform: translateZ(0);
          overflow: hidden;
          font-family: var(--font-sans, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
            "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Noto Sans CJK SC", sans-serif);
          font-size: 14px;
          line-height: 1.6;
          color: var(--color-text-primary);
          background: transparent;
          word-break: break-word;
        }
        img { max-width: 100%; height: auto; }
        a[href] { color: var(--color-email-link); }
        .pebble-email-content {
          box-sizing: border-box;
          position: relative;
          contain: layout paint;
          isolation: isolate;
          display: var(--email-content-display);
          max-width: 100%;
          overflow-x: auto;
          padding: var(--email-content-padding, 0);
          border-radius: var(--email-content-radius, 0);
          color-scheme: var(--email-content-color-scheme);
          color: var(--email-content-color);
          background: var(--email-content-bg);
        }
        .pebble-email-content[data-plain-text] {
          padding: 0;
          border-radius: 0;
          color-scheme: var(--app-color-scheme, light);
          color: var(--color-text-primary);
          background: transparent;
        }
        .pebble-plain-text-email {
          margin: 0;
          font: inherit;
          line-height: inherit;
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
          /* Derived from the surrounding text color so it reads on both the
             light HTML-mail card and themed plain-text content. */
          color: color-mix(in srgb, currentColor 62%, transparent);
          background: color-mix(in srgb, currentColor 6%, transparent);
          border: 1px dashed color-mix(in srgb, currentColor 32%, transparent);
          border-radius: 4px;
          text-align: center;
          max-width: 100%;
          box-sizing: border-box;
        }
      </style>
      <div class="pebble-email-content"${isPlainText ? " data-plain-text" : ""}>${safeHtml}</div>
    `;

    const handleClick = (event: Event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;

      const anchor = target.closest<HTMLAnchorElement>("a");
      if (!anchor) return;

      // Never let an email link navigate the app window: every anchor click is
      // handled here, and only absolute http(s) and mailto links go anywhere.
      event.preventDefault();
      const href = anchor.getAttribute("href")?.trim();
      if (!href) return;

      if (/^mailto:/i.test(href)) {
        void openMailtoUrl(href);
        return;
      }

      if (/^https?:\/\/[^/\\]/i.test(href)) {
        void invoke("open_external_url", { url: href })
          .catch((err) => console.warn("Failed to open email body link", err));
        return;
      }

      if (href.startsWith("#") && href.length > 1) {
        scrollToEmailFragment(shadow, href.slice(1));
      }
    };

    shadow.addEventListener("click", handleClick);
    return () => {
      shadow.removeEventListener("click", handleClick);
    };
  }, [html]);

  return <div ref={hostRef} className={className} />;
}
