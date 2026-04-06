import { useRef, useEffect } from "react";
import DOMPurify from "dompurify";

interface ShadowDomEmailProps {
  html: string;
  className?: string;
}

/** Sanitize HTML to prevent XSS while preserving email formatting */
function sanitizeHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS: [
      "a", "abbr", "address", "article", "b", "bdi", "bdo", "blockquote",
      "br", "caption", "cite", "code", "col", "colgroup", "dd", "del",
      "details", "dfn", "div", "dl", "dt", "em", "figcaption", "figure",
      "footer", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "i",
      "img", "ins", "kbd", "li", "main", "mark", "nav", "ol", "p", "pre",
      "q", "rp", "rt", "ruby", "s", "samp", "section", "small", "span",
      "strong", "sub", "summary", "sup", "table", "tbody", "td", "tfoot",
      "th", "thead", "time", "tr", "u", "ul", "var", "wbr", "center", "font",
    ],
    ALLOWED_ATTR: [
      "href", "src", "alt", "title", "width", "height", "style", "class",
      "dir", "id", "colspan", "rowspan", "border", "cellpadding", "cellspacing",
      "align", "valign", "bgcolor", "color", "face", "size",
    ],
    ALLOW_DATA_ATTR: false,
  });
}

export function ShadowDomEmail({ html, className }: ShadowDomEmailProps) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!hostRef.current) return;
    const shadow = hostRef.current.shadowRoot
      || hostRef.current.attachShadow({ mode: "open" });

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
        pre { white-space: pre-wrap; overflow-x: auto; }
        table { max-width: 100%; border-collapse: collapse; }
        body, div { word-wrap: break-word; overflow-wrap: break-word; }
      </style>
      <div>${sanitizeHtml(html)}</div>
    `;
  }, [html]);

  return <div ref={hostRef} className={className} />;
}
