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
        let body_html = extract_renderable_fragment(raw_html, mode);
        let embedded_style_preprocessed = preprocess_embedded_styles(&body_html, mode);
        let style_preprocessed = preprocess_stylesheet_links(&embedded_style_preprocessed, mode);

        // Pre-process images before ammonia sanitization
        let preprocessed = preprocess_images(
            &style_preprocessed,
            mode,
            &mut trackers_blocked,
            &mut images_blocked,
        );

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

fn extract_renderable_fragment(raw_html: &str, mode: &PrivacyMode) -> String {
    let body = extract_body_fragment(raw_html);
    if !looks_like_html_document(raw_html) {
        return body;
    }

    let mut head_css = String::new();
    if allow_embedded_styles(mode) {
        head_css.push_str(&collect_head_style_tags(raw_html));
    }
    if allow_external_stylesheets(mode) {
        head_css.push_str(&collect_head_stylesheet_links(raw_html));
    }

    if head_css.is_empty() {
        body
    } else {
        format!("{head_css}{body}")
    }
}

fn extract_body_fragment(raw_html: &str) -> String {
    if !looks_like_html_document(raw_html) {
        return raw_html.to_string();
    }

    if let Some(body_start) = find_ascii_case_insensitive(raw_html, "<body") {
        if let Some(open_end) = find_tag_end(&raw_html[body_start..]) {
            let content_start = body_start + open_end + 1;
            let body_tag = &raw_html[body_start..content_start];
            if let Some(close_start) =
                find_ascii_case_insensitive(&raw_html[content_start..], "</body")
            {
                return wrap_body_fragment(
                    body_tag,
                    &raw_html[content_start..content_start + close_start],
                );
            }
            return wrap_body_fragment(body_tag, &raw_html[content_start..]);
        }
    }

    strip_document_metadata_elements(raw_html)
}

fn allow_embedded_styles(mode: &PrivacyMode) -> bool {
    matches!(
        mode,
        PrivacyMode::LoadOnce | PrivacyMode::Off | PrivacyMode::TrustSender(_)
    )
}

fn allow_external_stylesheets(mode: &PrivacyMode) -> bool {
    matches!(mode, PrivacyMode::Off)
}

fn extract_head_fragment(raw_html: &str) -> Option<&str> {
    let head_start = find_ascii_case_insensitive(raw_html, "<head")?;
    let open_end = find_tag_end(&raw_html[head_start..])?;
    let content_start = head_start + open_end + 1;
    let close_start_rel = find_ascii_case_insensitive(&raw_html[content_start..], "</head")?;
    Some(&raw_html[content_start..content_start + close_start_rel])
}

fn collect_head_style_tags(raw_html: &str) -> String {
    let Some(head) = extract_head_fragment(raw_html) else {
        return String::new();
    };

    let mut output = String::new();
    let mut offset = 0;
    while let Some(start_rel) = find_ascii_case_insensitive(&head[offset..], "<style") {
        let start = offset + start_rel;
        let Some(open_end_rel) = find_tag_end(&head[start..]) else {
            break;
        };
        let content_start = start + open_end_rel + 1;
        let Some(close_start_rel) = find_ascii_case_insensitive(&head[content_start..], "</style")
        else {
            break;
        };
        let close_start = content_start + close_start_rel;
        let Some(close_end_rel) = find_tag_end(&head[close_start..]) else {
            break;
        };
        let end = close_start + close_end_rel + 1;
        output.push_str(&head[start..end]);
        offset = end;
    }
    output
}

fn collect_head_stylesheet_links(raw_html: &str) -> String {
    let Some(head) = extract_head_fragment(raw_html) else {
        return String::new();
    };

    let mut output = String::new();
    let mut offset = 0;
    while let Some(start_rel) = find_ascii_case_insensitive(&head[offset..], "<link") {
        let start = offset + start_rel;
        let Some(end_rel) = find_tag_end(&head[start..]) else {
            break;
        };
        let end = start + end_rel + 1;
        let tag = &head[start..end];
        if is_stylesheet_link_tag(tag) {
            output.push_str(tag);
        }
        offset = end;
    }
    output
}

fn is_stylesheet_link_tag(tag: &str) -> bool {
    let rel = html_attr_value(tag, "rel")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let href = html_attr_value(tag, "href").unwrap_or_default();
    is_stylesheet_link_attrs(&rel, &href)
}

fn is_stylesheet_link_attrs(rel: &str, href: &str) -> bool {
    rel.split_ascii_whitespace()
        .any(|token| token == "stylesheet")
        && (href.starts_with("https://") || href.starts_with("http://"))
}

fn html_attr_value(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'-' | b':' | b'_'))
        {
            index += 1;
        }
        if name_start == index {
            index += 1;
            continue;
        }
        let attr_name = &tag[name_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            return None;
        }
        let value = if bytes[index] == b'"' || bytes[index] == b'\'' {
            let quote = bytes[index];
            index += 1;
            let value_start = index;
            while index < bytes.len() && bytes[index] != quote {
                index += 1;
            }
            let value = tag[value_start..index].to_string();
            index = index.saturating_add(1);
            value
        } else {
            let value_start = index;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() && bytes[index] != b'>'
            {
                index += 1;
            }
            tag[value_start..index].to_string()
        };
        if attr_name.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

fn looks_like_html_document(html: &str) -> bool {
    find_ascii_case_insensitive(html, "<html").is_some()
        || find_ascii_case_insensitive(html, "<head").is_some()
        || find_ascii_case_insensitive(html, "<body").is_some()
        || find_ascii_case_insensitive(html, "<title").is_some()
        || find_ascii_case_insensitive(html, "<!doctype").is_some()
}

fn wrap_body_fragment(body_tag: &str, body_html: &str) -> String {
    let body_class = html_attr_value(body_tag, "class")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let body_style = body_style_from_attrs(body_tag);
    if body_class.is_none() && body_style.is_none() {
        return body_html.to_string();
    }

    let class_attr = match body_class {
        Some(class) => format!("pebble-email-body {class}"),
        None => "pebble-email-body".to_string(),
    };
    let style_attr = body_style
        .map(|style| format!(r#" style="{}""#, html_escape(&style)))
        .unwrap_or_default();

    format!(
        r#"<div class="{}"{}>{}</div>"#,
        html_escape(&class_attr),
        style_attr,
        body_html
    )
}

fn body_style_from_attrs(body_tag: &str) -> Option<String> {
    let mut style = html_attr_value(body_tag, "style")
        .map(|value| value.trim().trim_end_matches(';').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();

    if let Some(bgcolor) = html_attr_value(body_tag, "bgcolor")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let lower_style = style.to_ascii_lowercase();
        if !lower_style.contains("background") {
            if !style.is_empty() {
                style.push_str("; ");
            }
            style.push_str("background-color:");
            style.push_str(&bgcolor);
        }
    }

    if style.is_empty() {
        None
    } else {
        Some(style)
    }
}

fn strip_document_metadata_elements(html: &str) -> String {
    let without_head = strip_element_pair(html, "head");
    strip_element_pair(&without_head, "title")
}

fn strip_element_pair(html: &str, tag: &str) -> String {
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}");
    let mut stripped = String::with_capacity(html.len());
    let mut offset = 0;

    while let Some(start_rel) = find_ascii_case_insensitive(&html[offset..], &open_pattern) {
        let start = offset + start_rel;
        stripped.push_str(&html[offset..start]);

        let Some(close_start_rel) = find_ascii_case_insensitive(&html[start..], &close_pattern)
        else {
            if let Some(open_end_rel) = find_tag_end(&html[start..]) {
                offset = start + open_end_rel + 1;
                continue;
            }
            offset = html.len();
            break;
        };
        let close_start = start + close_start_rel;
        let Some(close_end_rel) = find_tag_end(&html[close_start..]) else {
            offset = html.len();
            break;
        };
        offset = close_start + close_end_rel + 1;
    }

    stripped.push_str(&html[offset..]);
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
        "border-radius",
        "border-style",
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
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
        "opacity",
        "overflow",
        "overflow-x",
        "overflow-y",
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

fn is_remote_loading_css(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("@import")
        || lower.contains("url(")
        || lower.contains("image-set(")
        || lower.contains("-webkit-image-set(")
        || lower.contains("cross-fade(")
        || lower.contains("element(")
        || lower.contains("paint(")
        || lower.contains("expression(")
        || lower.contains("javascript:")
        || lower.contains("vbscript:")
        || lower.contains("data:")
        || lower.contains('\\')
}

fn sanitize_embedded_css(css: &str) -> String {
    let without_imports = remove_css_imports(css);
    remove_remote_loading_rules(&without_imports)
}

fn remove_css_imports(css: &str) -> String {
    let mut output = String::new();
    let mut remaining = css;
    while let Some(import_start) = remaining.to_ascii_lowercase().find("@import") {
        output.push_str(&remaining[..import_start]);
        let after_import = &remaining[import_start..];
        if let Some(statement_end) = after_import.find(';') {
            remaining = &after_import[statement_end + 1..];
        } else {
            remaining = "";
            break;
        }
    }
    output.push_str(remaining);
    output
}

fn remove_remote_loading_rules(css: &str) -> String {
    let mut output = String::new();
    let mut offset = 0;
    while let Some(open_rel) = css[offset..].find('{') {
        let open = offset + open_rel;
        output.push_str(&css[offset..open]);
        if let Some(close_rel) = css[open + 1..].find('}') {
            let close = open + 1 + close_rel;
            let body = &css[open + 1..close];
            if !is_remote_loading_css(body) {
                output.push('{');
                output.push_str(body);
                output.push('}');
            }
            offset = close + 1;
        } else {
            output.push_str(&css[open..]);
            offset = css.len();
            break;
        }
    }
    output.push_str(&css[offset..]);
    output
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
fn build_sanitizer(mode: &PrivacyMode) -> Builder<'static> {
    let mut builder = Builder::new();

    // Allow safe tags for email HTML
    let mut tags: HashSet<&'static str> = [
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
    if allow_embedded_styles(mode) {
        tags.insert("style");
    }
    if allow_external_stylesheets(mode) {
        tags.insert("link");
    }

    builder.tags(tags);
    if allow_embedded_styles(mode) {
        builder.rm_clean_content_tags(&["style"]);
    }

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
            (
                "link",
                ["rel", "href", "type", "media"]
                    .iter()
                    .copied()
                    .collect::<HashSet<_>>(),
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
        ["http", "https", "mailto", "cid"]
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

fn preprocess_stylesheet_links(html: &str, mode: &PrivacyMode) -> String {
    if !allow_external_stylesheets(mode) {
        return html.to_string();
    }

    lol_html::rewrite_str(
        html,
        lol_html::RewriteStrSettings {
            element_content_handlers: vec![lol_html::element!("link", |el| {
                let rel = el
                    .get_attribute("rel")
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let href = el.get_attribute("href").unwrap_or_default();
                if !is_stylesheet_link_attrs(&rel, &href) {
                    el.remove();
                }
                Ok(())
            })],
            ..lol_html::RewriteStrSettings::default()
        },
    )
    .unwrap_or_else(|_| html.to_string())
}

fn preprocess_embedded_styles(html: &str, mode: &PrivacyMode) -> String {
    if !allow_embedded_styles(mode) || allow_external_stylesheets(mode) {
        return html.to_string();
    }

    rewrite_style_tag_contents(html, sanitize_embedded_css)
}

fn rewrite_style_tag_contents(html: &str, rewrite: fn(&str) -> String) -> String {
    let mut output = String::with_capacity(html.len());
    let mut offset = 0;
    while let Some(start_rel) = find_ascii_case_insensitive(&html[offset..], "<style") {
        let start = offset + start_rel;
        output.push_str(&html[offset..start]);
        let Some(open_end_rel) = find_tag_end(&html[start..]) else {
            output.push_str(&html[start..]);
            return output;
        };
        let content_start = start + open_end_rel + 1;
        output.push_str(&html[start..content_start]);
        let Some(close_start_rel) = find_ascii_case_insensitive(&html[content_start..], "</style")
        else {
            output.push_str(&rewrite(&html[content_start..]));
            return output;
        };
        let close_start = content_start + close_start_rel;
        output.push_str(&rewrite(&html[content_start..close_start]));
        let Some(close_end_rel) = find_tag_end(&html[close_start..]) else {
            output.push_str(&html[close_start..]);
            return output;
        };
        let close_end = close_start + close_end_rel + 1;
        output.push_str(&html[close_start..close_end]);
        offset = close_end;
    }
    output.push_str(&html[offset..]);
    output
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
    // Only explicitly disabling privacy bypasses image/tracker blocking.
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
                    // lol_html exposes text nodes in their raw, entity-encoded
                    // source form (e.g. `&amp;`). Decode to the logical text
                    // before scanning so the linkifier escapes exactly once
                    // instead of double-escaping existing entities, which would
                    // turn `&amp;` into `&amp;amp;` and surface a literal
                    // `&amp;` inside URLs (issue #68). On decode failure we
                    // leave the original chunk untouched rather than risk
                    // re-escaping it.
                    if let Ok(decoded) = htmlescape::decode_html(text.as_str()) {
                        if let Some(linked) = linkify_text_to_html(&decoded) {
                            text.replace(&linked, ContentType::Html);
                        }
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
    fn trust_sender_still_blocks_tracking_pixels() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracker.example.com/pixel.gif" width="1" height="1"><p>World</p>"#;
        let result = guard.render_safe_html(
            html,
            &PrivacyMode::TrustSender("trusted@example.com".to_string()),
        );
        assert!(!result.html.contains("tracker.example.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.images_blocked, 0);
    }

    #[test]
    fn trust_sender_still_blocks_known_tracker_domains() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracking.mailchimp.com/open.gif" width="100" height="50"><p>World</p>"#;
        let result = guard.render_safe_html(
            html,
            &PrivacyMode::TrustSender("trusted@example.com".to_string()),
        );
        assert!(!result.html.contains("mailchimp.com"));
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.images_blocked, 0);
    }

    #[test]
    fn trust_sender_allows_non_tracker_remote_images() {
        let guard = PrivacyGuard::new();
        let html = r#"<img src="https://example.com/photo.jpg" width="640" height="480">"#;
        let result = guard.render_safe_html(
            html,
            &PrivacyMode::TrustSender("trusted@example.com".to_string()),
        );

        assert!(result.html.contains("https://example.com/photo.jpg"));
        assert!(result.trackers_blocked.is_empty());
        assert_eq!(result.images_blocked, 0);
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
    fn test_preserves_hidden_preheader_clipping_styles() {
        let guard = PrivacyGuard::new();
        let html = r#"<div style="max-width:0px;max-height:0px;overflow-x:hidden;overflow-y:hidden;visibility:hidden;opacity:0">马凯，为您推荐 2 条新动态</div>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);

        assert!(result.html.contains("max-width:0px"));
        assert!(result.html.contains("max-height:0px"));
        assert!(result.html.contains("overflow-x:hidden"));
        assert!(result.html.contains("overflow-y:hidden"));
        assert!(result.html.contains("visibility:hidden"));
        assert!(result.html.contains("opacity:0"));
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
    fn render_safe_html_drops_title_text_from_fragment_documents() {
        let guard = PrivacyGuard::new();
        let html = r#"<!doctype html><title>Leaked subject</title><div>Visible body</div>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);

        assert!(result.html.contains("Visible body"));
        assert!(!result.html.contains("Leaked subject"));
    }

    #[test]
    fn render_safe_html_preserves_body_container_styles() {
        let guard = PrivacyGuard::new();
        let html = r#"
            <html>
              <body style="margin:0; padding:48px 0; background-color:#e6e9ed">
                <table style="border-radius:20px; background-color:#ffffff">
                  <tr><td>Visible body</td></tr>
                </table>
              </body>
            </html>
        "#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);

        assert!(result.html.contains("pebble-email-body"));
        assert!(result.html.contains("padding:48px 0"));
        assert!(result.html.contains("background-color:#e6e9ed"));
        assert!(result.html.contains("border-radius:20px"));
        assert!(result.html.contains("Visible body"));
    }

    #[test]
    fn load_once_preserves_embedded_style_tags_without_leaking_head_text() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><title>Leaked subject</title><style>.hero{color:red}</style></head><body><style>.cta{font-weight:bold}</style><p class="hero cta">Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);

        assert!(result.html.contains("<style>"));
        assert!(result.html.contains(".hero{color:red}"));
        assert!(result.html.contains(".cta{font-weight:bold}"));
        assert!(result.html.contains("Visible body"));
        assert!(!result.html.contains("Leaked subject"));
    }

    #[test]
    fn load_once_drops_external_stylesheet_links() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><link rel="stylesheet" href="https://cdn.example.com/mail.css"></head><body><p>Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);

        assert!(!result.html.contains("cdn.example.com"));
        assert!(!result.html.contains("<link"));
        assert!(result.html.contains("Visible body"));
    }

    #[test]
    fn trust_sender_drops_external_stylesheet_links() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><link rel="stylesheet" href="https://cdn.example.com/mail.css"></head><body><p>Visible body</p></body></html>"#;
        let result = guard.render_safe_html(
            html,
            &PrivacyMode::TrustSender("trusted@example.com".to_string()),
        );

        assert!(!result.html.contains("cdn.example.com"));
        assert!(!result.html.contains("<link"));
        assert!(result.html.contains("Visible body"));
    }

    #[test]
    fn load_once_filters_remote_loads_from_embedded_style_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><style>@import url("https://cdn.example.com/mail.css"); .hero{color:red} .tracking{background:url("https://tracker.example.com/pixel")}</style></head><body><p class="hero tracking">Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::LoadOnce);

        assert!(result.html.contains(".hero{color:red}"));
        assert!(!result.html.contains("@import"));
        assert!(!result.html.contains("cdn.example.com"));
        assert!(!result.html.contains("tracker.example.com"));
    }

    #[test]
    fn off_preserves_external_stylesheet_links() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><link rel="stylesheet" href="https://cdn.example.com/mail.css"></head><body><p>Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Off);

        assert!(result.html.contains(r#"rel="stylesheet""#));
        assert!(result.html.contains("https://cdn.example.com/mail.css"));
        assert!(result.html.contains("Visible body"));
    }

    #[test]
    fn off_preserves_remote_loads_in_embedded_style_tags() {
        let guard = PrivacyGuard::new();
        let html = r#"<html><head><style>@import url("https://cdn.example.com/mail.css"); .tracking{background:url("https://tracker.example.com/pixel")}</style></head><body><p class="tracking">Visible body</p></body></html>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Off);

        assert!(result.html.contains("@import"));
        assert!(result.html.contains("cdn.example.com"));
        assert!(result.html.contains("tracker.example.com"));
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
    fn linkify_does_not_double_escape_ampersands_in_plain_text_urls() {
        // Regression for issue #68: a query-string URL shown as visible text
        // (not an existing anchor) must not have its `&` double-escaped into a
        // literal `&amp;` in the rendered link.
        let guard = PrivacyGuard::new();

        // HTML email path: the URL appears as text; the source encodes `&` as
        // `&amp;` per HTML rules.
        let html = guard.render_message_html(
            r#"<p>Copy this link: https://example.com/wp-login.php?login=a&amp;key=b&amp;action=rp</p>"#,
            "",
            &PrivacyMode::Strict,
        );
        assert!(
            !html.html.contains("&amp;amp;"),
            "double-escaped ampersand in HTML path: {}",
            html.html
        );
        assert!(
            html.html.contains(
                r#"<a href="https://example.com/wp-login.php?login=a&amp;key=b&amp;action=rp" target="_blank" rel="noopener noreferrer">https://example.com/wp-login.php?login=a&amp;key=b&amp;action=rp</a>"#
            ),
            "unexpected anchor markup in HTML path: {}",
            html.html
        );

        // Plain-text email path: body text has literal `&` which is escaped
        // once when wrapped in <pre>, and must not be escaped again.
        let text = guard.render_message_html(
            "",
            "Copy this link: https://example.com/wp-login.php?login=a&key=b&action=rp",
            &PrivacyMode::Strict,
        );
        assert!(
            !text.html.contains("&amp;amp;"),
            "double-escaped ampersand in plain-text path: {}",
            text.html
        );
        assert!(
            text.html.contains(
                r#"<a href="https://example.com/wp-login.php?login=a&amp;key=b&amp;action=rp" target="_blank" rel="noopener noreferrer">https://example.com/wp-login.php?login=a&amp;key=b&amp;action=rp</a>"#
            ),
            "unexpected anchor markup in plain-text path: {}",
            text.html
        );
    }

    #[test]
    fn linkify_still_works_around_common_entities() {
        // Ensure decoding does not regress linkification when the chunk also
        // contains common email entities (nbsp, copy, numeric).
        let guard = PrivacyGuard::new();
        let result = guard.render_message_html(
            "<p>Visit https://example.com/a?x=1&amp;y=2&nbsp;now &copy; 2026 &#169;</p>",
            "",
            &PrivacyMode::Strict,
        );
        assert!(
            !result.html.contains("&amp;amp;"),
            "double-escaped ampersand: {}",
            result.html
        );
        assert!(
            result
                .html
                .contains(r#"<a href="https://example.com/a?x=1&amp;y=2""#),
            "URL was not linkified alongside entities: {}",
            result.html
        );
        // The copyright entity must survive (not be dropped or double-escaped).
        assert!(
            result.html.contains('\u{00A9}')
                || result.html.contains("&copy;")
                || result.html.contains("&#169;"),
            "copyright entity lost: {}",
            result.html
        );
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
