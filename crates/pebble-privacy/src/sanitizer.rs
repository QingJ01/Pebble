use std::collections::HashSet;

use ammonia::Builder;
use pebble_core::{PrivacyMode, RenderedHtml, TrackerInfo};

use crate::tracker::{is_known_tracker, is_tracking_pixel};

pub struct PrivacyGuard;

impl PrivacyGuard {
    pub fn new() -> Self {
        Self
    }

    pub fn render_message_html(
        &self,
        raw_html: &str,
        body_text: &str,
        mode: &PrivacyMode,
    ) -> RenderedHtml {
        let source_html = if raw_html.trim().is_empty() && !body_text.is_empty() {
            format!(
                r#"<pre class="pebble-plain-text-email">{}</pre>"#,
                html_escape(body_text)
            )
        } else {
            raw_html.to_string()
        };

        let mut rendered = self.render_safe_html(&source_html, mode);
        rendered.html = linkify_html_text_nodes(&rendered.html);
        rendered
    }

    pub fn render_safe_html(&self, raw_html: &str, mode: &PrivacyMode) -> RenderedHtml {
        let mut trackers_blocked: Vec<TrackerInfo> = Vec::new();
        let mut images_blocked: u32 = 0;
        let body_html = extract_body_fragment(raw_html);

        // Pre-process images before ammonia sanitization
        let preprocessed =
            preprocess_images(&body_html, mode, &mut trackers_blocked, &mut images_blocked);

        // Sanitize with ammonia
        let sanitizer = build_sanitizer(mode);
        let clean_html = sanitizer.clean(&preprocessed).to_string();

        RenderedHtml {
            html: clean_html,
            trackers_blocked,
            images_blocked,
        }
    }
}

impl Default for PrivacyGuard {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_body_fragment(raw_html: &str) -> String {
    if !looks_like_html_document(raw_html) {
        return raw_html.to_string();
    }

    if let Some(body_start) = find_ascii_case_insensitive(raw_html, "<body") {
        if let Some(open_end) = find_tag_end(&raw_html[body_start..]) {
            let content_start = body_start + open_end + 1;
            if let Some(close_start) =
                find_ascii_case_insensitive(&raw_html[content_start..], "</body")
            {
                return raw_html[content_start..content_start + close_start].to_string();
            }
            return raw_html[content_start..].to_string();
        }
    }

    strip_head_element(raw_html)
}

fn looks_like_html_document(html: &str) -> bool {
    find_ascii_case_insensitive(html, "<html").is_some()
        || find_ascii_case_insensitive(html, "<head").is_some()
        || find_ascii_case_insensitive(html, "<body").is_some()
}

fn strip_head_element(html: &str) -> String {
    let Some(head_start) = find_ascii_case_insensitive(html, "<head") else {
        return html.to_string();
    };
    let Some(close_start_rel) = find_ascii_case_insensitive(&html[head_start..], "</head") else {
        return html.to_string();
    };
    let close_start = head_start + close_start_rel;
    let Some(close_end_rel) = find_tag_end(&html[close_start..]) else {
        return html.to_string();
    };
    let close_end = close_start + close_end_rel + 1;

    let mut stripped = String::with_capacity(html.len().saturating_sub(close_end - head_start));
    stripped.push_str(&html[..head_start]);
    stripped.push_str(&html[close_end..]);
    stripped
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn find_tag_end(html: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (idx, ch) in html.char_indices() {
        match quote {
            Some(current) if ch == current => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '>' => return Some(idx),
            None => {}
        }
    }
    None
}

/// Parse a CSS style string and keep only properties from the safe allowlist.
fn filter_css_properties(style: &str) -> String {
    const SAFE_PROPERTIES: &[&str] = &[
        "color",
        "background",
        "background-color",
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "font-variant",
        "text-align",
        "text-decoration",
        "text-indent",
        "text-transform",
        "line-height",
        "letter-spacing",
        "word-spacing",
        "white-space",
        "vertical-align",
        "direction",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "padding",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
        "border-color",
        "border-style",
        "border-width",
        "border-collapse",
        "border-spacing",
        "width",
        "max-width",
        "min-width",
        "height",
        "max-height",
        "min-height",
        "display",
        "overflow",
        "visibility",
        "float",
        "clear",
        "list-style",
        "list-style-type",
        "table-layout",
    ];

    style
        .split(';')
        .filter_map(|decl| {
            let decl = decl.trim();
            if decl.is_empty() {
                return None;
            }
            let colon = decl.find(':')?;
            let prop = decl[..colon].trim().to_lowercase();
            let value = decl[colon + 1..].trim().to_lowercase();
            if !SAFE_PROPERTIES.contains(&prop.as_str()) {
                return None;
            }
            if prop == "background" && !is_safe_background_shorthand_value(&value) {
                return None;
            }
            // Reject URL/script-bearing values and CSS escapes that can hide them.
            if value.contains("url(")
                || value.contains("image-set(")
                || value.contains("-webkit-image-set(")
                || value.contains("cross-fade(")
                || value.contains("element(")
                || value.contains("paint(")
                || value.contains("expression(")
                || value.contains("javascript:")
                || value.contains("vbscript:")
                || value.contains("data:")
                || value.contains("@import")
                || value.contains('\\')
            {
                return None;
            }
            Some(decl.to_string())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn is_safe_background_shorthand_value(value: &str) -> bool {
    let trimmed = value.trim();
    let without_important = trimmed
        .strip_suffix("!important")
        .map(str::trim)
        .unwrap_or(trimmed);
    let value = without_important.to_lowercase();
    if value.is_empty()
        || value.contains("url(")
        || value.contains("image-set(")
        || value.contains("-webkit-image-set(")
        || value.contains("cross-fade(")
        || value.contains("element(")
        || value.contains("paint(")
        || value.contains("expression(")
        || value.contains("javascript:")
        || value.contains("vbscript:")
        || value.contains("data:")
        || value.contains("@import")
        || value.contains('\\')
    {
        return false;
    }

    matches!(value.as_str(), "none" | "transparent" | "currentcolor")
        || is_hex_color(&value)
        || is_css_color_function(&value)
        || value.chars().all(|c| c.is_ascii_alphabetic())
}

fn is_hex_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_css_color_function(value: &str) -> bool {
    let Some(open_paren) = value.find('(') else {
        return false;
    };
    if !value.ends_with(')') {
        return false;
    }
    let function = &value[..open_paren];
    if !matches!(function, "rgb" | "rgba" | "hsl" | "hsla") {
        return false;
    }
    value[open_paren + 1..value.len() - 1]
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, ' ' | '\t' | '.' | ',' | '%' | '/' | '+' | '-'))
}

/// Build an ammonia sanitizer configured for safe email HTML rendering.
fn build_sanitizer(_mode: &PrivacyMode) -> Builder<'static> {
    let mut builder = Builder::new();

    // Allow safe tags for email HTML
    let tags: HashSet<&'static str> = [
        "a",
        "abbr",
        "b",
        "blockquote",
        "br",
        "code",
        "dd",
        "div",
        "dl",
        "dt",
        "em",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "hr",
        "i",
        "img",
        "li",
        "ol",
        "p",
        "pre",
        "s",
        "span",
        "strong",
        "sub",
        "sup",
        "table",
        "tbody",
        "td",
        "th",
        "thead",
        "tr",
        "u",
        "ul",
        "center",
        "font",
    ]
    .iter()
    .copied()
    .collect();

    builder.tags(tags);

    // Configure per-tag attributes
    builder.tag_attributes(
        [
            (
                "a",
                ["href", "title", "target"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "img",
                ["src", "alt", "width", "height", "class"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "td",
                ["colspan", "rowspan", "align", "valign"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "th",
                ["colspan", "rowspan", "align", "valign"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "table",
                ["border", "cellpadding", "cellspacing", "width", "align"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "font",
                ["color", "size", "face"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "div",
                ["class", "data-src"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
            ),
            (
                "blockquote",
                ["cite"].iter().copied().collect::<HashSet<_>>(),
            ),
        ]
        .iter()
        .cloned()
        .collect(),
    );

    // Generic attributes allowed on all tags
    builder.generic_attributes(
        ["style", "class", "dir", "id"]
            .iter()
            .copied()
            .collect::<HashSet<_>>(),
    );

    // Only allow safe URL schemes (blocks javascript:, data:, vbscript:, etc.)
    builder.url_schemes(
        ["http", "https", "mailto"]
            .iter()
            .copied()
            .collect::<HashSet<_>>(),
    );

    // Add rel="noopener noreferrer" to all links
    builder.link_rel(Some("noopener noreferrer"));

    // Filter style attributes using a CSS property allowlist
    builder.attribute_filter(|_element, attribute, value| {
        if attribute == "style" {
            let filtered = filter_css_properties(value);
            if filtered.is_empty() {
                None
            } else {
                Some(filtered.into())
            }
        } else {
            Some(value.into())
        }
    });

    builder
}

/// Pre-process img tags before ammonia to handle tracking pixels and privacy modes.
///
/// Uses lol_html (a streaming HTML rewriter) to parse `<img>` elements
/// properly, avoiding the pitfalls of hand-rolled string scanning (attribute
/// quoting, whitespace variations, encoding tricks).
fn preprocess_images(
    html: &str,
    mode: &PrivacyMode,
    trackers_blocked: &mut Vec<TrackerInfo>,
    images_blocked: &mut u32,
) -> String {
    use std::cell::RefCell;

    // Wrap mutable references in RefCell so they can be captured by the
    // closure passed to lol_html (which requires 'static-compatible FnMut).
    let trackers = RefCell::new(trackers_blocked);
    let blocked = RefCell::new(images_blocked);
    let mode = mode.clone();

    let result = lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("img", |el| {
                let src = el.get_attribute("src");
                let width = el.get_attribute("width");
                let height = el.get_attribute("height");

                let action = process_img_tag(
                    src.as_deref(),
                    width.as_deref(),
                    height.as_deref(),
                    &mode,
                    &mut trackers.borrow_mut(),
                    &mut blocked.borrow_mut(),
                );

                match action {
                    ImgAction::Remove => {
                        el.remove();
                    }
                    ImgAction::BlockedPlaceholder => {
                        let src_val = src.as_deref().unwrap_or("");
                        let alt_val = el.get_attribute("alt").unwrap_or_default();
                        let label = if alt_val.trim().is_empty() {
                            "Image blocked for privacy".to_string()
                        } else {
                            alt_val
                        };
                        let escaped_src = html_escape(src_val);
                        let escaped_label = html_escape(&label);
                        el.replace(
                            &format!(
                                r#"<div class="blocked-image" data-src="{}">{}</div>"#,
                                escaped_src, escaped_label
                            ),
                            lol_html::html_content::ContentType::Html,
                        );
                    }
                    ImgAction::Keep => { /* leave element untouched */ }
                }

                Ok(())
            })],
            ..lol_html::RewriteStrSettings::default()
        },
    );

    match result {
        Ok(rewritten) => rewritten,
        Err(_) => {
            // If the rewriter fails on malformed input, fall through to
            // ammonia which will strip the problematic markup anyway.
            html.to_string()
        }
    }
}

enum ImgAction {
    Remove,
    BlockedPlaceholder,
    Keep,
}

fn process_img_tag(
    src: Option<&str>,
    width: Option<&str>,
    height: Option<&str>,
    mode: &PrivacyMode,
    trackers_blocked: &mut Vec<TrackerInfo>,
    images_blocked: &mut u32,
) -> ImgAction {
    // Off mode: no blocking at all
    if matches!(mode, PrivacyMode::Off) {
        return ImgAction::Keep;
    }

    // Tracking pixels are always blocked
    if is_tracking_pixel(width, height) {
        let domain = src.and_then(extract_domain_from_url).unwrap_or_default();
        trackers_blocked.push(TrackerInfo {
            domain,
            tracker_type: "pixel".to_string(),
        });
        return ImgAction::Remove;
    }

    // Known tracker domains are always blocked
    if let Some(src_val) = src {
        if let Some(domain) = extract_domain_from_url(src_val) {
            if is_known_tracker(&domain) {
                trackers_blocked.push(TrackerInfo {
                    domain,
                    tracker_type: "domain".to_string(),
                });
                return ImgAction::Remove;
            }
        }

        // External images depend on privacy mode
        let is_external = src_val.starts_with("http://") || src_val.starts_with("https://");
        if is_external {
            match mode {
                PrivacyMode::Strict => {
                    *images_blocked += 1;
                    return ImgAction::BlockedPlaceholder;
                }
                PrivacyMode::LoadOnce | PrivacyMode::TrustSender(_) | PrivacyMode::Off => {
                    return ImgAction::Keep;
                }
            }
        }
    }

    ImgAction::Keep
}

/// Extract the domain from a URL, stripping protocol and path.
fn extract_domain_from_url(url: &str) -> Option<String> {
    let without_protocol = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    let domain = without_protocol
        .split('/')
        .next()
        .unwrap_or(without_protocol);
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_string())
    }
}

/// Escape special HTML characters for use in attribute values.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn linkify_html_text_nodes(html: &str) -> String {
    use lol_html::html_content::ContentType;
    use std::cell::RefCell;
    use std::rc::Rc;

    let anchor_depth = Rc::new(RefCell::new(0usize));
    let anchor_depth_for_element = Rc::clone(&anchor_depth);
    let anchor_depth_for_text = Rc::clone(&anchor_depth);

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("a", move |el| {
                *anchor_depth_for_element.borrow_mut() += 1;
                let anchor_depth = Rc::clone(&anchor_depth_for_element);
                if let Some(handlers) = el.end_tag_handlers() {
                    handlers.push(Box::new(move |_| {
                        let mut depth = anchor_depth.borrow_mut();
                        *depth = depth.saturating_sub(1);
                        Ok(())
                    }));
                }
                Ok(())
            })],
            document_content_handlers: vec![lol_html::doc_text!(move |text| {
                if *anchor_depth_for_text.borrow() == 0 {
                    if let Some(linked) = linkify_text_to_html(text.as_str()) {
                        text.replace(&linked, ContentType::Html);
                    }
                }
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::default()
        },
    )
    .unwrap_or_else(|_| html.to_string())
}

fn linkify_text_to_html(text: &str) -> Option<String> {
    let mut output = String::new();
    let mut last_copied = 0usize;
    let mut index = 0usize;
    let mut changed = false;

    while index < text.len() {
        if starts_with_http_url(text, index) {
            let raw_end = scan_url_end(text, index);
            let link_end = trim_url_end(text, raw_end);
            if link_end > index {
                output.push_str(&html_escape(&text[last_copied..index]));
                append_anchor(&mut output, &text[index..link_end], &text[index..link_end]);
                last_copied = link_end;
                index = link_end;
                changed = true;
                continue;
            }
        }

        if let Some(email_end) = scan_email_end(text, index) {
            let email = &text[index..email_end];
            output.push_str(&html_escape(&text[last_copied..index]));
            append_anchor(&mut output, &format!("mailto:{email}"), email);
            last_copied = email_end;
            index = email_end;
            changed = true;
            continue;
        }

        index = next_char_index(text, index);
    }

    if changed {
        output.push_str(&html_escape(&text[last_copied..]));
        Some(output)
    } else {
        None
    }
}

fn append_anchor(output: &mut String, href: &str, label: &str) {
    output.push_str(r#"<a href=""#);
    output.push_str(&html_escape(href));
    output.push_str(r#"" target="_blank" rel="noopener noreferrer">"#);
    output.push_str(&html_escape(label));
    output.push_str("</a>");
}

fn starts_with_http_url(text: &str, index: usize) -> bool {
    text[index..].starts_with("http://") || text[index..].starts_with("https://")
}

fn scan_url_end(text: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, ch) in text[start..].char_indices() {
        if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'') {
            break;
        }
        end = start + offset + ch.len_utf8();
    }
    end
}

fn trim_url_end(text: &str, mut end: usize) -> usize {
    while let Some(ch) = text[..end].chars().last() {
        if matches!(ch, '.' | ',' | '!' | '?' | ':' | ';' | ')' | ']' | '}') {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }
    end
}

fn scan_email_end(text: &str, start: usize) -> Option<usize> {
    if start > 0 {
        let previous = text[..start].chars().last()?;
        if is_email_local_char(previous) || previous == '@' {
            return None;
        }
    }

    let mut index = start;
    let mut local_len = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if !is_email_local_char(ch) {
            break;
        }
        local_len += ch.len_utf8();
        index += ch.len_utf8();
    }

    if local_len == 0 || !text[index..].starts_with('@') {
        return None;
    }
    index += 1;

    let domain_start = index;
    let mut has_dot = false;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if !is_email_domain_char(ch) {
            break;
        }
        if ch == '.' {
            has_dot = true;
        }
        index += ch.len_utf8();
    }

    while index > domain_start {
        let ch = text[..index].chars().last()?;
        if matches!(ch, '.' | '-') {
            index -= ch.len_utf8();
        } else {
            break;
        }
    }

    let domain = &text[domain_start..index];
    if !has_dot || !domain_has_valid_labels(domain) {
        return None;
    }

    Some(index)
}

fn domain_has_valid_labels(domain: &str) -> bool {
    let mut labels = domain.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let mut saw_tld = false;
    for label in labels {
        if label.is_empty() {
            return false;
        }
        saw_tld = true;
        if label.len() >= 2 && label.chars().all(|ch| ch.is_ascii_alphabetic()) {
            return true;
        }
    }
    saw_tld
}

fn is_email_local_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-')
}

fn is_email_domain_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')
}

fn next_char_index(text: &str, index: usize) -> usize {
    index
        + text[index..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_removes_script_tags() {
        let guard = PrivacyGuard::new();
        let html = "<p>Hello</p><script>alert('xss')</script><p>World</p>";
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("script"));
        assert!(!result.html.contains("alert"));
        assert!(result.html.contains("Hello"));
        assert!(result.html.contains("World"));
    }

    #[test]
    fn test_removes_event_handlers() {
        let guard = PrivacyGuard::new();
        let html = r#"<p onmouseover="alert(1)">Hello</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("onmouseover"));
        assert!(result.html.contains("Hello"));
    }

    #[test]
    fn test_blocks_javascript_urls() {
        let guard = PrivacyGuard::new();
        let html = r#"<a href="javascript:alert(1)">Click me</a>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("javascript:"));
        assert!(result.html.contains("Click me"));
    }

    #[test]
    fn test_removes_iframe_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Before</p><iframe src="https://evil.com">content</iframe><p>After</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("iframe"));
        assert!(result.html.contains("Before"));
        assert!(result.html.contains("After"));
    }

    #[test]
    fn test_removes_style_tags() {
        let guard = PrivacyGuard::new();
        let html = "<p>Hello</p><style>body { color: red; }</style><p>World</p>";
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("<style>"));
        assert!(!result.html.contains("color: red"));
        assert!(result.html.contains("Hello"));
        assert!(result.html.contains("World"));
    }

    #[test]
    fn test_blocks_tracking_pixel() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracker.example.com/pixel.gif" width="1" height="1"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("tracker.example.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.trackers_blocked[0].tracker_type, "pixel");
    }

    #[test]
    fn test_blocks_known_tracker_domain() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracking.mailchimp.com/open.gif" width="100" height="50"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("mailchimp.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.trackers_blocked[0].tracker_type, "domain");
    }

    #[test]
    fn test_blocks_external_images_in_strict_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://example.com/photo.jpg"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("blocked-image"));
        assert_eq!(result.images_blocked, 1);
    }

    #[test]
    fn test_allows_images_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://example.com/photo.jpg"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert!(result.html.contains("https://example.com/photo.jpg"));
        assert_eq!(result.images_blocked, 0);
    }

    #[test]
    fn test_still_blocks_trackers_in_load_once_mode() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://tracking.mailchimp.com/open.gif" width="100" height="50">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);
        assert!(!result.html.contains("mailchimp.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
    }

    #[test]
    fn test_removes_svg_with_event_handlers() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Before</p><svg onload="alert(1)"><circle r="10"/></svg><p>After</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("onload"));
        assert!(!result.html.contains("svg"));
        assert!(result.html.contains("Before"));
        assert!(result.html.contains("After"));
    }

    #[test]
    fn test_blocks_css_url_exfiltration() {
        let guard = PrivacyGuard::new();
        let html = r#"<p style="background: url('https://evil.com/steal')">text</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("evil.com"));
    }

    #[test]
    fn test_blocks_escaped_css_url_exfiltration() {
        let guard = PrivacyGuard::new();
        let html = r#"<p style="background: u\72l('https://evil.com/steal')">text</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("evil.com"));
    }

    #[test]
    fn test_blocks_css_import() {
        let guard = PrivacyGuard::new();
        let html = r#"<div style="@import url('https://evil.com/exfil.css')">text</div>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("evil.com"));
    }

    #[test]
    fn test_allows_safe_css_properties() {
        let guard = PrivacyGuard::new();
        let html = r#"<p style="color: red; font-size: 14px; margin: 10px">text</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("color: red"));
        assert!(result.html.contains("font-size: 14px"));
    }

    #[test]
    fn test_allows_safe_background_shorthand_for_email_buttons() {
        let guard = PrivacyGuard::new();
        let html = r##"<a style="background: #f38020; color: #ffffff; border: 1px solid #f38020">Open dashboard</a>"##;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(result.html.contains("background: #f38020"));
        assert!(result.html.contains("color: #ffffff"));
    }

    #[test]
    fn render_safe_html_uses_body_fragment_from_full_documents() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><title>Leaked subject</title><style>p{color:red}</style></head><body><p>Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);

        assert!(result.html.contains("Visible body"));
        assert!(!result.html.contains("Leaked subject"));
        assert!(!result.html.contains("p{color:red}"));
    }

    #[test]
    fn test_blocks_position_properties() {
        let guard = PrivacyGuard::new();
        let html = r#"<div style="position: fixed; top: 0; left: 0; z-index: 9999">overlay</div>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(!result.html.contains("position"));
        assert!(!result.html.contains("z-index"));
    }

    #[test]
    fn img_tag_end_respects_quoted_gt() {
        // The alt attribute contains a '>' inside quotes. The naive parser
        // that looks for the first '>' would close the tag early, leaving
        // a stray src=".../pixel.gif" fragment in the output.
        let guard = PrivacyGuard::new();
        let html = r#"<p>Before</p><img alt="hi>there" src="https://tracking.mailchimp.com/open.gif" width="100" height="50"><p>After</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        // Tracker must be detected and the src must not survive in output.
        assert!(
            !result.html.contains("mailchimp.com"),
            "tracker src leaked: {}",
            result.html
        );
        assert_eq!(result.trackers_blocked.len(), 1);
    }

    #[test]
    fn extract_attr_does_not_match_substring() {
        // `data-src` should NOT be treated as `src`. A substring-matching
        // parser would pull the data-src value and miss the real src.
        let guard = PrivacyGuard::new();
        let html = r#"<img data-src="https://example.com/local.jpg" src="https://tracking.mailchimp.com/open.gif" width="100" height="50">"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert!(
            !result.html.contains("mailchimp.com"),
            "tracker leaked: {}",
            result.html
        );
        assert_eq!(
            result.trackers_blocked.len(),
            1,
            "expected real src to be detected"
        );
    }

    #[test]
    fn render_message_html_linkifies_plain_text_urls_and_emails() {
        let guard = PrivacyGuard::new();
        let result = guard.render_message_html(
            "",
            "Visit https://example.com/path and contact support@example.com.",
            &PrivacyMode::Strict,
        );

        assert!(result
            .html
            .contains(r#"<a href="https://example.com/path" target="_blank" rel="noopener noreferrer">https://example.com/path</a>"#));
        assert!(result
            .html
            .contains(r#"<a href="mailto:support@example.com" target="_blank" rel="noopener noreferrer">support@example.com</a>"#));
        assert!(result.html.contains("<pre"));
    }

    #[test]
    fn render_message_html_linkifies_html_text_nodes() {
        let guard = PrivacyGuard::new();
        let result = guard.render_message_html(
            "<p>Open https://example.com or mail team@example.org</p>",
            "",
            &PrivacyMode::Strict,
        );

        assert!(result
            .html
            .contains(r#"<a href="https://example.com" target="_blank" rel="noopener noreferrer">https://example.com</a>"#));
        assert!(result
            .html
            .contains(r#"<a href="mailto:team@example.org" target="_blank" rel="noopener noreferrer">team@example.org</a>"#));
    }

    #[test]
    fn render_message_html_does_not_wrap_existing_links_again() {
        let guard = PrivacyGuard::new();
        let result = guard.render_message_html(
            r#"<p><a href="https://example.com">https://example.com</a> support@example.com</p>"#,
            "",
            &PrivacyMode::Strict,
        );

        assert_eq!(
            result
                .html
                .matches(r#"<a href="https://example.com""#)
                .count(),
            1
        );
        assert!(result
            .html
            .contains(r#"<a href="mailto:support@example.com" target="_blank" rel="noopener noreferrer">support@example.com</a>"#));
    }
}
