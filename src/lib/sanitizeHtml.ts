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

/**
 * Whether an email link may keep its href. Only absolute http(s) and mailto
 * links can leave the email; in-document fragments are handled inside the
 * reader. Relative and protocol-relative ("//host") hrefs would resolve against
 * the app origin and navigate the main window, so they are dropped.
 */
export function isAllowedEmailLinkHref(href: string): boolean {
  return /^https?:\/\/[^/\\]/i.test(href) || /^mailto:/i.test(href) || /^#/.test(href);
}

function normalizeLinkAttributes(html: string): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  template.content.querySelectorAll<HTMLAnchorElement>("a[href]").forEach((anchor) => {
    const href = anchor.getAttribute("href")?.trim() ?? "";
    if (!isAllowedEmailLinkHref(href)) {
      anchor.removeAttribute("href");
      anchor.removeAttribute("target");
      anchor.removeAttribute("rel");
    } else if (/^(https?:|mailto:)/i.test(href)) {
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

  template.content.querySelectorAll("style").forEach((style) => {
    // `querySelectorAll("style")` also matches SVG/MathML-namespace <style>,
    // whose children are parsed as markup. Only HTML raw-text style elements
    // carry plain CSS; anything else is dropped (DOMPurify would drop it too).
    if (style.namespaceURI === HTML_NAMESPACE && style.children.length === 0) {
      cssNodes.push(`<style>${escapeCssText(style.textContent ?? "")}</style>`);
    }
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
    cssNodes: verifyCssNodes(cssNodes.join("")),
    htmlWithoutCssNodes: template.innerHTML,
  };
}

const HTML_NAMESPACE = "http://www.w3.org/1999/xhtml";

/**
 * CSS never needs a literal "<"; escaping it as a CSS hex escape keeps string
 * values intact while making it impossible to close the <style> element.
 */
function escapeCssText(css: string): string {
  return css.replace(/</g, "\\3c ");
}

/**
 * The CSS nodes are concatenated outside DOMPurify, so re-parse them and make
 * sure the result is nothing but top-level <style>/<link> elements with text.
 */
function verifyCssNodes(cssNodes: string): string {
  if (!cssNodes) return "";
  const template = document.createElement("template");
  template.innerHTML = cssNodes;
  const nodes = Array.from(template.content.childNodes);
  const safe = nodes.every((node) => {
    if (!(node instanceof Element) || node.namespaceURI !== HTML_NAMESPACE) return false;
    const tag = node.localName;
    if (tag === "link") return true;
    return tag === "style" && node.children.length === 0;
  });
  return safe ? cssNodes : "";
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

/** Class the compose view puts on its own reply-quote wrapper. */
export const COMPOSE_QUOTE_CLASS = "compose-original-quote";

const LAYOUT_ESCAPING_PROPERTIES = new Set([
  "position", "inset", "top", "right", "bottom", "left", "z-index", "transform", "translate",
]);

function stripLayoutEscapingStyles(style: string): string {
  return style
    .split(";")
    .map((part) => part.trim())
    .filter(Boolean)
    .filter((part) => {
      const name = part.split(":")[0].trim().toLowerCase();
      return !LAYOUT_ESCAPING_PROPERTIES.has(name) && !name.startsWith("inset-");
    })
    .join("; ");
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
  // Quotes render in the app's light DOM: app utility classes / ids must not
  // apply, and positioning must not let the quote escape its container.
  template.content.querySelectorAll<HTMLElement>("[class], [id]").forEach((element) => {
    element.removeAttribute("id");
    const isQuoteWrapper = element.localName === "blockquote"
      && element.getAttribute("class") === COMPOSE_QUOTE_CLASS;
    if (!isQuoteWrapper) element.removeAttribute("class");
  });
  template.content.querySelectorAll<HTMLElement>("[style]").forEach((element) => {
    const filtered = stripLayoutEscapingStyles(element.getAttribute("style") ?? "");
    if (filtered) {
      element.setAttribute("style", filtered);
    } else {
      element.removeAttribute("style");
    }
  });
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
