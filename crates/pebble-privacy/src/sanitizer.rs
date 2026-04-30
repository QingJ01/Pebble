use std::collections::HashSet;

use ammonia::Builder;
use pebble_core::{PrivacyMode, RenderedHtml, TrackerInfo};

use crate::tracker::{is_known_tracker, is_tracking_pixel};

pub struct PrivacyGuard;

impl PrivacyGuard {
    pub fn new() -> Self {
        Self
    }

    pub fn render_safe_html(&self, raw_html: &str, mode: &PrivacyMode) -> RenderedHtml {
        let mut trackers_blocked: Vec<TrackerInfo> = Vec::new();
        let mut images_blocked: u32 = 0;

        // Pre-process images before ammonia sanitization
        let preprocessed =
            preprocess_images(raw_html, mode, &mut trackers_blocked, &mut images_blocked);

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

/// Parse a CSS style string and keep only properties from the safe allowlist.
fn filter_css_properties(style: &str) -> String {
    const SAFE_PROPERTIES: &[&str] = &[
        "color",
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
            // Reject URL/script-bearing values and CSS escapes that can hide them.
            if value.contains("url(")
                || value.contains("expression(")
                || value.contains("javascript:")
                || value.contains("vbscript:")
                || value.contains('\\')
            {
                return None;
            }
            Some(decl.to_string())
        })
        .collect::<Vec<_>>()
        .join("; ")
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
                        let escaped = html_escape(src_val);
                        el.replace(
                            &format!(
                                r#"<div class="blocked-image" data-src="{}">Image blocked for privacy</div>"#,
                                escaped
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
}
