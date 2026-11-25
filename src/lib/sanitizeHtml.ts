import DOMPurify from "dompurify";

const SAFE_STYLE_PROPERTIES = new Set([
  "background",
  "background-color",
  "border",
  "border-bottom",
  "border-bottom-left-radius",
  "border-bottom-right-radius",
  "border-collapse",
  "border-color",
  "border-left",
  "border-radius",
  "border-right",
  "border-spacing",
  "border-style",
  "border-top",
  "border-top-left-radius",
  "border-top-right-radius",
  "border-width",
  "color",
  "display",
  "font",
  "font-family",
  "font-size",
  "font-style",
  "font-weight",
  "height",
  "line-height",
  "margin",
  "margin-bottom",
  "margin-left",
  "margin-right",
  "margin-top",
  "max-height",
  "max-width",
  "min-height",
  "min-width",
  "opacity",
  "overflow",
  "overflow-x",
  "overflow-y",
  "padding",
  "padding-bottom",
  "padding-left",
  "padding-right",
  "padding-top",
  "text-align",
  "text-decoration",
  "vertical-align",
  "visibility",
  "white-space",
  "width",
]);

function isSafeBackgroundShorthandValue(value: string): boolean {
  const normalized = value
    .trim()
    .replace(/\s*!important\s*$/i, "")
    .trim()
    .toLowerCase();

  if (!normalized) return false;
  if (
    /(url\s*\(|image-set\s*\(|-webkit-image-set\s*\(|cross-fade\s*\(|element\s*\(|paint\s*\(|expression\s*\(|javascript:|vbscript:|data:|@import|\\)/i.test(
      normalized,
    )
  ) {
    return false;
  }
  if (["none", "transparent", "currentcolor"].includes(normalized)) return true;
  if (/^#[0-9a-f]{3,8}$/i.test(normalized)) return true;
  if (/^(rgb|rgba|hsl|hsla)\([\d\s.,%/+-]+\)$/i.test(normalized)) return true;
  return /^[a-z]+$/.test(normalized);
}

function filterStyleAttribute(style: string): string {
  return style
    .split(";")
    .map((part) => part.trim())
    .filter(Boolean)
    .filter((part) => {
      const [rawName, ...rawValue] = part.split(":");
      const name = rawName.trim().toLowerCase();
      const value = rawValue.join(":").trim().toLowerCase();
      if (!SAFE_STYLE_PROPERTIES.has(name) || !value) return false;
      if (name === "background") return isSafeBackgroundShorthandValue(value);
      if (value.includes("\\")) return false;
      return !/(url\s*\(|expression\s*\(|javascript:|vbscript:|data:|@import)/i.test(value);
    })
    .join("; ");
}

function filterInlineStyles(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content.querySelectorAll<HTMLElement>("[style]").forEach((element) => {
    const filtered = filterStyleAttribute(element.getAttribute("style") ?? "");
    if (filtered) {
      element.setAttribute("style", filtered);
    } else {
      element.removeAttribute("style");
    }
  });
  return template.innerHTML;
}

function normalizeLinkAttributes(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((anchor) => {
    const href = anchor.getAttribute("href")?.trim() ?? "";
    if (/^(https?:|mailto:)/i.test(href)) {
      anchor.setAttribute("target", "_blank");
      anchor.setAttribute("rel", "noopener noreferrer");
    } else {
      anchor.removeAttribute("target");
      anchor.removeAttribute("rel");
    }
  });
  return template.innerHTML;
}

function filterStylesheetLinks(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content.querySelectorAll<HTMLLinkElement>("link").forEach((link) => {
    const rel = link.getAttribute("rel")?.toLowerCase() ?? "";
    const href = link.getAttribute("href")?.trim() ?? "";
    const isStylesheet = rel.split(/\s+/).includes("stylesheet");
    const isHttp = /^https?:\/\//i.test(href);
    if (!isStylesheet || !isHttp) {
      link.remove();
    }
  });
  return template.innerHTML;
}

function looksLikeFullHtmlDocument(html: string): boolean {
  return /<\s*(html|head|body)\b/i.test(html);
}

function escapeAttribute(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function extractApprovedCssNodes(html: string): { cssNodes: string; htmlWithoutCssNodes: string } {
  if (looksLikeFullHtmlDocument(html)) {
    return { cssNodes: "", htmlWithoutCssNodes: html };
  }

  const template = document.createElement("template");
  template.innerHTML = html;
  const cssNodes: string[] = [];

  template.content.querySelectorAll<HTMLStyleElement>("style").forEach((style) => {
    cssNodes.push(`<style>${style.textContent ?? ""}</style>`);
    style.remove();
  });

  template.content.querySelectorAll<HTMLLinkElement>("link").forEach((link) => {
    const rel = link.getAttribute("rel")?.toLowerCase() ?? "";
    const href = link.getAttribute("href")?.trim() ?? "";
    const isStylesheet = rel.split(/\s+/).includes("stylesheet");
    const isHttp = /^https?:\/\//i.test(href);
    if (isStylesheet && isHttp) {
      const type = link.getAttribute("type")?.trim();
      const media = link.getAttribute("media")?.trim();
      cssNodes.push(
        `<link rel="stylesheet" href="${escapeAttribute(href)}"` +
          (type ? ` type="${escapeAttribute(type)}"` : "") +
          (media ? ` media="${escapeAttribute(media)}"` : "") +
          ">",
      );
    }
    link.remove();
  });

  return {
    cssNodes: cssNodes.join(""),
    htmlWithoutCssNodes: template.innerHTML,
  };
}

/** Sanitize HTML to prevent XSS while preserving email formatting. */
export function sanitizeHtml(html: string): string {
  const { cssNodes, htmlWithoutCssNodes } = extractApprovedCssNodes(html);
  const sanitized = DOMPurify.sanitize(htmlWithoutCssNodes, {
    ALLOWED_TAGS: [
      "a", "abbr", "address", "article", "b", "bdi", "bdo", "blockquote",
      "br", "caption", "center", "cite", "code", "col", "colgroup", "dd", "del",
      "details", "dfn", "div", "dl", "dt", "em", "figcaption", "figure",
      "font", "footer", "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "i",
      "img", "ins", "kbd", "li", "main", "mark", "nav", "ol", "p", "pre",
      "q", "rp", "rt", "ruby", "s", "samp", "section", "small", "span",
      "strong", "sub", "summary", "sup", "table", "tbody", "td", "tfoot",
      "th", "thead", "time", "tr", "u", "ul", "var", "wbr",
    ],
    ALLOWED_ATTR: [
      "href", "src", "alt", "title", "width", "height", "class",
      "target", "rel",
      "dir", "id", "lang", "colspan", "rowspan", "border", "cellpadding",
      "cellspacing", "align", "valign", "bgcolor", "color", "face", "size",
      "style",
    ],
    ALLOW_DATA_ATTR: false,
  });
  return cssNodes + normalizeLinkAttributes(filterStylesheetLinks(filterInlineStyles(sanitized)));
}

/**
 * Sanitize HTML before placing an untrusted message inside the compose UI.
 * Unlike the isolated message renderer, compose quotes live in the app's light DOM,
 * so document-level CSS and remotely loaded resources must not be retained.
 */
export function sanitizeComposeQuoteHtml(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = sanitizeHtml(html);

  template.content.querySelectorAll("style, link").forEach((element) => element.remove());
  template.content.querySelectorAll<HTMLElement>("[src], [srcset], [background], [poster]").forEach((element) => {
    element.removeAttribute("srcset");
    for (const attribute of ["src", "background", "poster"]) {
      const value = element.getAttribute(attribute) ?? "";
      const normalized = value.replace(/[\u0000-\u0020]+/g, "").toLowerCase();
      if (!normalized.startsWith("data:")) {
        element.removeAttribute(attribute);
      }
    }
  });

  return template.innerHTML;
}
