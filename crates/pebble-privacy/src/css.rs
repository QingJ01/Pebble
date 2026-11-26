//! A small, conservative CSS tokenizer/filter for embedded `<style>` content.
//!
//! The filter re-serialises a stylesheet rule by rule and declaration by
//! declaration. Comments are removed first (respecting strings and escapes),
//! so tricks such as `.x{/*}*/background:url(https://t)}` cannot hide a
//! declaration from the filter. As a final safety net, every emitted piece
//! (selector, at-rule prelude, declaration) is checked again for anything
//! that could trigger a network load, so a parsing divergence between this
//! filter and the browser can only ever cause content to be *dropped*.

use crate::tracker::is_known_tracker;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CssPolicy {
    /// Drop everything that could trigger a remote load (`url()`,
    /// `image-set()`, `@import`, `@font-face` sources, ...).
    BlockRemote,
    /// Keep remote loads except those that target known tracker domains.
    BlockTrackers,
}

/// Functions / tokens that can make the engine fetch a resource or run code.
const LOADING_PATTERNS: &[&str] = &[
    "url(",
    "src(",
    "image(",
    "image-set(",
    "cross-fade(",
    "element(",
    "paint(",
    "attr(",
    "expression(",
    "javascript:",
    "vbscript:",
    "@import",
];

/// At-rules whose block contains a list of rules.
const RULE_LIST_AT_RULES: &[&str] = &[
    "media",
    "supports",
    "layer",
    "container",
    "keyframes",
    "-webkit-keyframes",
    "-moz-keyframes",
];

/// At-rules whose block contains a list of declarations.
const DECLARATION_AT_RULES: &[&str] = &["font-face", "page", "counter-style", "property"];

/// Deepest block nesting (at-rules and nested selectors combined) kept from
/// untrusted CSS. Deeper blocks are dropped, which bounds the recursion below
/// so a hostile stylesheet cannot overflow the rendering thread's stack.
/// Real-world mail CSS rarely nests more than three or four levels.
const MAX_NESTING_DEPTH: usize = 32;

/// Properties that are never allowed in embedded CSS.
const FORBIDDEN_PROPERTIES: &[&str] = &["behavior", "-moz-binding"];

pub(crate) fn sanitize_embedded_css(css: &str, policy: CssPolicy) -> String {
    let stripped = strip_comments(css);
    filter_rule_list(&stripped, policy, 0)
}

/// Remove `/* ... */` comments outside of strings. Each comment is replaced
/// by a single space, which matches how the CSS tokenizer separates tokens.
fn strip_comments(css: &str) -> String {
    let chars: Vec<char> = css.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push(c);
            if let Some(next) = chars.get(i + 1) {
                out.push(*next);
            }
            i += 2;
            continue;
        }
        match quote {
            Some(q) => {
                if c == q || c == '\n' {
                    quote = None;
                }
                out.push(c);
                i += 1;
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    out.push(c);
                    i += 1;
                } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                    let mut j = i + 2;
                    while j < chars.len() && !(chars[j] == '*' && chars.get(j + 1) == Some(&'/')) {
                        j += 1;
                    }
                    i = (j + 2).min(chars.len());
                    out.push(' ');
                } else {
                    out.push(c);
                    i += 1;
                }
            }
        }
    }
    out
}

struct Cursor {
    chars: Vec<char>,
    pos: usize,
}

impl Cursor {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_whitespace_and(&mut self, extra: &[char]) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() || extra.contains(&c) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Read until one of `stops` at paren depth zero, honouring strings and
    /// escapes. The stop character is consumed and returned.
    fn consume_until(&mut self, stops: &[char]) -> (String, Option<char>) {
        let mut out = String::new();
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' => {
                    out.push(c);
                    if let Some(next) = self.peek() {
                        out.push(next);
                        self.pos += 1;
                    }
                }
                '"' | '\'' => {
                    out.push(c);
                    self.consume_string_body(c, &mut out);
                }
                '(' | '[' => {
                    depth += 1;
                    out.push(c);
                }
                ')' | ']' => {
                    depth = depth.saturating_sub(1);
                    out.push(c);
                }
                _ if depth == 0 && stops.contains(&c) => return (out, Some(c)),
                _ => out.push(c),
            }
        }
        (out, None)
    }

    fn consume_string_body(&mut self, quote: char, out: &mut String) {
        while let Some(c) = self.peek() {
            self.pos += 1;
            out.push(c);
            match c {
                '\\' => {
                    if let Some(next) = self.peek() {
                        out.push(next);
                        self.pos += 1;
                    }
                }
                '\n' => return,
                _ if c == quote => return,
                _ => {}
            }
        }
    }

    /// Called right after a `{` was consumed: returns the block contents up
    /// to the matching `}` (which is consumed).
    fn consume_block(&mut self) -> String {
        let mut out = String::new();
        let mut depth = 0usize;
        while let Some(c) = self.peek() {
            self.pos += 1;
            match c {
                '\\' => {
                    out.push(c);
                    if let Some(next) = self.peek() {
                        out.push(next);
                        self.pos += 1;
                    }
                }
                '"' | '\'' => {
                    out.push(c);
                    self.consume_string_body(c, &mut out);
                }
                '{' => {
                    depth += 1;
                    out.push(c);
                }
                '}' => {
                    if depth == 0 {
                        return out;
                    }
                    depth -= 1;
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        out
    }

    fn consume_at_keyword(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '\\' {
                name.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        name
    }
}

fn filter_rule_list(text: &str, policy: CssPolicy, depth: usize) -> String {
    if depth >= MAX_NESTING_DEPTH {
        return String::new();
    }
    let mut cursor = Cursor::new(text);
    let mut out = String::new();
    loop {
        cursor.skip_whitespace_and(&[';', '}']);
        if cursor.eof() {
            break;
        }
        if cursor.peek() == Some('@') {
            cursor.pos += 1;
            let name = cursor.consume_at_keyword();
            let (prelude, stop) = cursor.consume_until(&['{', ';']);
            let block = (stop == Some('{')).then(|| cursor.consume_block());
            emit_at_rule(&name, &prelude, block.as_deref(), policy, depth, &mut out);
        } else {
            let (prelude, stop) = cursor.consume_until(&['{', ';']);
            if stop != Some('{') {
                // A rule without a block is invalid; drop it.
                continue;
            }
            let block = cursor.consume_block();
            let prelude = prelude.trim();
            if prelude.is_empty() || !is_safe_piece(prelude, policy) {
                continue;
            }
            out.push_str(prelude);
            out.push('{');
            out.push_str(&filter_declaration_list(&block, policy, depth + 1));
            out.push('}');
        }
    }
    out
}

fn emit_at_rule(
    raw_name: &str,
    prelude: &str,
    block: Option<&str>,
    policy: CssPolicy,
    depth: usize,
    out: &mut String,
) {
    if raw_name.is_empty() || raw_name.contains('\\') {
        return;
    }
    let name = raw_name.to_ascii_lowercase();
    let prelude = prelude.trim();

    match block {
        None => {
            let keep = match name.as_str() {
                "charset" | "layer" => is_safe_piece(prelude, policy),
                "import" => {
                    policy == CssPolicy::BlockTrackers
                        && !references_known_tracker(prelude)
                        && !has_escaped_function(prelude)
                }
                _ => false,
            };
            if keep {
                out.push('@');
                out.push_str(&name);
                if !prelude.is_empty() {
                    out.push(' ');
                    out.push_str(prelude);
                }
                out.push(';');
            }
        }
        Some(block) => {
            if !is_safe_piece(prelude, policy) {
                return;
            }
            let inner = if RULE_LIST_AT_RULES.contains(&name.as_str()) {
                filter_rule_list(block, policy, depth + 1)
            } else if DECLARATION_AT_RULES.contains(&name.as_str()) {
                filter_declaration_list(block, policy, depth + 1)
            } else {
                return;
            };
            out.push('@');
            out.push_str(&name);
            if !prelude.is_empty() {
                out.push(' ');
                out.push_str(prelude);
            }
            out.push('{');
            out.push_str(&inner);
            out.push('}');
        }
    }
}

fn filter_declaration_list(text: &str, policy: CssPolicy, depth: usize) -> String {
    if depth >= MAX_NESTING_DEPTH {
        return String::new();
    }
    let mut cursor = Cursor::new(text);
    let mut kept: Vec<String> = Vec::new();
    loop {
        cursor.skip_whitespace_and(&[';', '}']);
        if cursor.eof() {
            break;
        }
        if cursor.peek() == Some('@') {
            // Nested at-rules inside a declaration block are dropped.
            let (_, stop) = cursor.consume_until(&['{', ';']);
            if stop == Some('{') {
                cursor.consume_block();
            }
            continue;
        }
        let (piece, stop) = cursor.consume_until(&['{', ';']);
        if stop == Some('{') {
            // CSS nesting: `selector { declarations }`.
            let block = cursor.consume_block();
            let selector = piece.trim();
            if !selector.is_empty() && is_safe_piece(selector, policy) {
                kept.push(format!(
                    "{selector}{{{}}}",
                    filter_declaration_list(&block, policy, depth + 1)
                ));
            }
            continue;
        }
        if let Some(decl) = filter_declaration(&piece, policy) {
            kept.push(decl);
        }
    }
    kept.join(";")
}

fn filter_declaration(decl: &str, policy: CssPolicy) -> Option<String> {
    let colon = decl.find(':')?;
    let name = decl[..colon].trim().to_ascii_lowercase();
    let value = decl[colon + 1..].trim();
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    if FORBIDDEN_PROPERTIES.contains(&name.as_str()) {
        return None;
    }
    if name == "position" {
        let lower = value.to_ascii_lowercase();
        if lower.contains("fixed") || lower.contains("sticky") || lower.contains('\\') {
            return None;
        }
    }
    if !is_safe_piece(value, policy) {
        return None;
    }
    Some(format!("{name}:{value}"))
}

/// Final check applied to every piece of text that is copied to the output.
fn is_safe_piece(text: &str, policy: CssPolicy) -> bool {
    if has_escaped_function(text) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("expression(")
        || lower.contains("javascript:")
        || lower.contains("vbscript:")
        || lower.contains("</style")
    {
        return false;
    }
    match policy {
        CssPolicy::BlockRemote => !LOADING_PATTERNS.iter().any(|p| lower.contains(p)),
        CssPolicy::BlockTrackers => !references_known_tracker(&lower),
    }
}

/// Escapes combined with a function call can spell out `url(` and friends
/// (e.g. `u\72l(`), which a substring check cannot see.
fn has_escaped_function(text: &str) -> bool {
    text.contains('\\') && text.contains('(')
}

/// True when the text references a URL whose host is a known tracker.
fn references_known_tracker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut search = lower.as_str();
    while let Some(idx) = search.find("//") {
        let after = &search[idx + 2..];
        let authority: String = after
            .chars()
            .take_while(|c| !matches!(c, '/' | '?' | '#' | '"' | '\'' | ')' | ' ' | '\\'))
            .collect();
        let host = authority.rsplit('@').next().unwrap_or("");
        let host = host.split(':').next().unwrap_or("");
        if !host.is_empty() && is_known_tracker(host) {
            return true;
        }
        search = after;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strict(css: &str) -> String {
        sanitize_embedded_css(css, CssPolicy::BlockRemote)
    }

    #[test]
    fn keeps_plain_rules() {
        assert_eq!(strict(".hero{color:red}"), ".hero{color:red}");
        assert_eq!(
            strict("@media (max-width: 600px) { .a { color: red; } }"),
            "@media (max-width: 600px){.a{color:red}}"
        );
    }

    #[test]
    fn comment_cannot_hide_remote_url() {
        let out = strict(".x{/*}*/background:url(https://t.example/p)}");
        assert!(!out.contains("url("), "{out}");
        assert!(!out.contains("t.example"), "{out}");
    }

    #[test]
    fn drops_url_but_keeps_sibling_declarations() {
        let out = strict(".x{color:red;background:URL( 'https://t.example/p' );margin:0}");
        assert_eq!(out, ".x{color:red;margin:0}");
    }

    #[test]
    fn drops_imports_and_image_set_and_escapes() {
        let out = strict(
            "@import 'https://a.example/x.css'; @IMPORT url(https://a.example/y.css);\
             .a{background-image:image-set('https://b.example/i.png' 1x)}\
             .b{background:u\\72l(https://c.example/i.png)}\
             @\\69mport 'https://d.example/z.css';",
        );
        for host in ["a.example", "b.example", "c.example", "d.example"] {
            assert!(!out.contains(host), "{host} leaked: {out}");
        }
    }

    #[test]
    fn drops_font_face_remote_sources() {
        let out =
            strict("@font-face{font-family:X;src:url(https://f.example/x.woff2) format('woff2')}");
        assert!(!out.contains("f.example"), "{out}");
        assert!(out.contains("font-family:X"), "{out}");
    }

    #[test]
    fn string_containing_brace_does_not_desync() {
        let out = strict(".a{content:'}'} .b{background:url(https://t.example/p)}");
        assert!(!out.contains("t.example"), "{out}");
    }

    #[test]
    fn drops_nested_rules_with_remote_urls() {
        let out = strict(".a{color:red; .b{background:url(https://t.example/p)}}");
        assert!(!out.contains("t.example"), "{out}");
        assert!(out.contains("color:red"), "{out}");
    }

    #[test]
    fn strips_fixed_and_sticky_positioning() {
        let out = strict(".a{position:fixed;top:0}.b{POSITION: Sticky}.c{position:relative}");
        assert!(!out.to_ascii_lowercase().contains("fixed"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("sticky"), "{out}");
        assert!(out.contains("position:relative"), "{out}");
        let out = sanitize_embedded_css(".a{position:fixed}", CssPolicy::BlockTrackers);
        assert!(!out.contains("fixed"), "{out}");
    }

    #[test]
    fn tracker_policy_keeps_remote_but_blocks_known_trackers() {
        let out = sanitize_embedded_css(
            ".a{background:url(https://cdn.example.com/a.png)}\
             .b{background:url(https://x.list-manage.com/open.gif)}\
             @import url('https://user@mailchimp.com:443/x.css');",
            CssPolicy::BlockTrackers,
        );
        assert!(out.contains("cdn.example.com"), "{out}");
        assert!(!out.contains("list-manage.com"), "{out}");
        assert!(!out.contains("mailchimp.com"), "{out}");
    }

    #[test]
    fn unterminated_comment_and_block_are_safe() {
        assert!(!strict(".a{color:red} /* .b{background:url(https://t/x)}").contains("url("));
        assert!(!strict(".a{background:url(https://t/x)").contains("url("));
    }

    #[test]
    fn deeply_nested_css_is_truncated_without_overflowing_the_stack() {
        // Run on a small stack so unbounded recursion would abort the test.
        let handle = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let depth = 20_000;
                let mut css = "@media screen{".repeat(depth);
                css.push_str("a{color:red}");
                css.push_str(&"}".repeat(depth));
                let mut nested = "a{".repeat(depth);
                nested.push_str("color:red");
                nested.push_str(&"}".repeat(depth));
                (strict(&css), strict(&nested))
            })
            .unwrap();
        let (media, selectors) = handle.join().expect("sanitizer must not overflow");
        assert!(media.matches("@media").count() <= MAX_NESTING_DEPTH);
        assert!(
            !media.contains("color:red"),
            "content past the limit is dropped"
        );
        assert!(selectors.matches('{').count() <= MAX_NESTING_DEPTH);

        // Ordinary nesting is untouched.
        assert_eq!(
            strict("@media screen{@supports (display:grid){a{color:red}}}"),
            "@media screen{@supports (display:grid){a{color:red}}}"
        );
    }
}
