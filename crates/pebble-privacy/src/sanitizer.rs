use pebble_core::{PrivacyMode, RenderedHtml, TrackerInfo};

use crate::tracker::{is_known_tracker, is_tracking_pixel};

pub struct PrivacyGuard {
    custom_tracker_domains: Vec<String>,
}

impl PrivacyGuard {
    pub fn new() -> Self {
        Self {
            custom_tracker_domains: Vec::new(),
        }
    }

    pub fn add_custom_tracker(&mut self, domain: impl Into<String>) {
        self.custom_tracker_domains.push(domain.into());
    }

    pub fn render_safe_html(&self, raw_html: &str, mode: &PrivacyMode) -> RenderedHtml {
        let mut result = String::with_capacity(raw_html.len());
        let mut trackers_blocked: Vec<TrackerInfo> = Vec::new();
        let mut images_blocked: u32 = 0;

        let chars: Vec<char> = raw_html.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            if chars[i] == '<' {
                // Find the end of this tag
                let tag_start = i;
                let mut tag_end = i + 1;
                while tag_end < len && chars[tag_end] != '>' {
                    tag_end += 1;
                }
                if tag_end >= len {
                    // Unclosed tag, output as-is
                    result.push(chars[i]);
                    i += 1;
                    continue;
                }

                let tag_str: String = chars[tag_start..=tag_end].iter().collect();
                let tag_lower = tag_str.to_lowercase();

                // Check for dangerous tags that should be stripped with content
                const STRIP_TAGS: &[&str] =
                    &["script", "iframe", "form", "object", "embed"];

                let mut stripped = false;
                for &strip_tag in STRIP_TAGS {
                    let open_pattern = format!("<{}", strip_tag);
                    if tag_lower.starts_with(&open_pattern)
                        && (tag_lower.len() > open_pattern.len()
                            && (chars[tag_start + open_pattern.len()] == ' '
                                || chars[tag_start + open_pattern.len()] == '>'
                                || chars[tag_start + open_pattern.len()] == '/'))
                    {
                        // Self-closing tag check
                        if tag_lower.ends_with("/>") {
                            i = tag_end + 1;
                            stripped = true;
                            break;
                        }

                        // Find closing tag
                        let close_tag = format!("</{}>", strip_tag);
                        let remaining: String = chars[tag_end + 1..].iter().collect();
                        let remaining_lower = remaining.to_lowercase();
                        if let Some(close_pos) = remaining_lower.find(&close_tag) {
                            i = tag_end + 1 + close_pos + close_tag.len();
                        } else {
                            // No closing tag found, skip just the opening tag
                            i = tag_end + 1;
                        }
                        stripped = true;
                        break;
                    }
                }

                if stripped {
                    continue;
                }

                // Handle <img> tags
                if tag_lower.starts_with("<img")
                    && (tag_lower.len() > 4
                        && (chars[tag_start + 4] == ' '
                            || chars[tag_start + 4] == '>'
                            || chars[tag_start + 4] == '/'))
                {
                    let src = extract_attr(&tag_str, "src");
                    let width = extract_attr(&tag_str, "width");
                    let height = extract_attr(&tag_str, "height");

                    // Check for tracking pixel
                    if is_tracking_pixel(width.as_deref(), height.as_deref()) {
                        let domain = src
                            .as_deref()
                            .map(extract_domain)
                            .unwrap_or_default();
                        trackers_blocked.push(TrackerInfo {
                            domain,
                            tracker_type: "pixel".to_string(),
                        });
                        i = tag_end + 1;
                        continue;
                    }

                    // Check for known tracker domain
                    if let Some(ref src_val) = src {
                        let domain = extract_domain(src_val);
                        if is_known_tracker(&domain) || self.is_custom_tracker(&domain) {
                            trackers_blocked.push(TrackerInfo {
                                domain,
                                tracker_type: "domain".to_string(),
                            });
                            i = tag_end + 1;
                            continue;
                        }

                        // External image handling based on mode
                        let is_external =
                            src_val.starts_with("http://") || src_val.starts_with("https://");
                        if is_external {
                            match mode {
                                PrivacyMode::Strict => {
                                    let escaped_src = html_escape(src_val);
                                    result.push_str(&format!(
                                        "<div class=\"blocked-image\" data-src=\"{}\">Image blocked for privacy</div>",
                                        escaped_src
                                    ));
                                    images_blocked += 1;
                                    i = tag_end + 1;
                                    continue;
                                }
                                PrivacyMode::LoadOnce | PrivacyMode::TrustSender(_) => {
                                    // Allow through
                                }
                            }
                        }
                    }

                    result.push_str(&tag_str);
                    i = tag_end + 1;
                    continue;
                }

                // All other tags pass through
                result.push_str(&tag_str);
                i = tag_end + 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }

        RenderedHtml {
            html: result,
            trackers_blocked,
            images_blocked,
        }
    }

    fn is_custom_tracker(&self, domain: &str) -> bool {
        let domain_lower = domain.to_lowercase();
        self.custom_tracker_domains
            .iter()
            .any(|d| domain_lower.contains(&d.to_lowercase()))
    }
}

impl Default for PrivacyGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the value of an attribute from an HTML tag string.
fn extract_attr(tag: &str, name: &str) -> Option<String> {
    let tag_lower = tag.to_lowercase();
    let search = format!("{}=", name);

    let attr_pos = tag_lower.find(&search)?;
    let value_start = attr_pos + search.len();
    let bytes = tag.as_bytes();

    if value_start >= bytes.len() {
        return None;
    }

    if bytes[value_start] == b'"' || bytes[value_start] == b'\'' {
        let quote = bytes[value_start];
        let start = value_start + 1;
        let end = tag[start..].find(|c: char| c as u8 == quote).map(|p| p + start)?;
        Some(tag[start..end].to_string())
    } else {
        // Unquoted attribute value — ends at space or >
        let start = value_start;
        let end = tag[start..]
            .find([' ', '>', '/'])
            .map(|p| p + start)
            .unwrap_or(tag.len());
        Some(tag[start..end].to_string())
    }
}

/// Extract the domain from a URL, stripping protocol and path.
fn extract_domain(url: &str) -> String {
    let without_protocol = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);

    without_protocol
        .split('/')
        .next()
        .unwrap_or(without_protocol)
        .to_string()
}

/// Escape special HTML characters.
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
        assert_eq!(result.html, "<p>Hello</p><p>World</p>");
    }

    #[test]
    fn test_blocks_tracking_pixel() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracker.example.com/pixel.gif" width="1" height="1"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert_eq!(result.html, "<p>Hello</p><p>World</p>");
        assert_eq!(result.trackers_blocked.len(), 1);
        assert_eq!(result.trackers_blocked[0].tracker_type, "pixel");
    }

    #[test]
    fn test_blocks_known_tracker_domain() {
        let guard = PrivacyGuard::new();
        let html = r#"<p>Hello</p><img src="https://tracking.mailchimp.com/open.gif" width="100" height="50"><p>World</p>"#;
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert_eq!(result.html, "<p>Hello</p><p>World</p>");
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
        assert_eq!(result.html, "");
        assert_eq!(result.trackers_blocked.len(), 1);
    }

    #[test]
    fn test_removes_iframe_tags() {
        let guard = PrivacyGuard::new();
        let html = "<p>Before</p><iframe src=\"https://evil.com\">content</iframe><p>After</p>";
        let result = guard.render_safe_html(html, &PrivacyMode::Strict);
        assert_eq!(result.html, "<p>Before</p><p>After</p>");
    }
}
