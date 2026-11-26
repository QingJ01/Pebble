use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders, PartType};
use pebble_core::{build_snippet, EmailAddress, PebbleError, Result};

/// Metadata about a parsed attachment.
#[derive(Debug, Clone)]
pub struct AttachmentMeta {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// Attachment metadata together with the raw binary content.
#[derive(Debug, Clone)]
pub struct AttachmentData {
    pub meta: AttachmentMeta,
    pub data: Vec<u8>,
}

/// The result of parsing a raw email message.
#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub date: i64,
    pub body_text: String,
    pub body_html: String,
    pub snippet: String,
    pub has_attachments: bool,
    pub attachments: Vec<AttachmentData>,
}

/// Convert a mail-parser `Address` into a `Vec<EmailAddress>`.
fn address_to_list(addr: Option<&Address<'_>>) -> Vec<EmailAddress> {
    let Some(addr) = addr else {
        return Vec::new();
    };
    addr.iter()
        .map(|a| EmailAddress {
            name: a.name().map(|s| s.to_string()),
            address: a.address().unwrap_or("").to_string(),
        })
        .collect()
}

/// Extract message IDs from a header value that holds a list of IDs.
/// Returns them joined by a space.
/// mail-parser strips the angle brackets from message IDs, while
/// `message_id_header` is stored as `<id>`. Re-bracket references so thread
/// lookups compare like with like.
fn bracket_message_id(id: &str) -> Option<String> {
    let bare = id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!bare.is_empty()).then(|| format!("<{bare}>"))
}

fn extract_id_list(hv: &HeaderValue<'_>) -> Option<String> {
    let raw: Vec<&str> = match hv {
        HeaderValue::Text(s) => vec![s.as_ref()],
        HeaderValue::TextList(list) => list.iter().map(|s| s.as_ref()).collect(),
        _ => return None,
    };
    let ids: Vec<String> = raw
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter_map(bracket_message_id)
        .collect();
    (!ids.is_empty()).then(|| ids.join(" "))
}

/// Maximum number of attachment parts kept from one message.
pub const MAX_ATTACHMENT_PARTS: usize = 500;
/// Maximum total decoded attachment bytes kept from one message.
pub const MAX_TOTAL_ATTACHMENT_BYTES: usize = 100 * 1024 * 1024;

/// Take ownership of a part body. Decoded (base64/quoted-printable) bodies
/// are already owned by the parser and move out without copying; only
/// bodies borrowed from the raw message are copied once.
fn part_body_into_bytes(body: PartType<'_>) -> Vec<u8> {
    match body {
        PartType::Text(text) | PartType::Html(text) => text.into_owned().into_bytes(),
        PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.into_owned(),
        // An unencoded message/rfc822 part borrows the *outer* email in
        // `raw_message`; the accessor slices out just the nested message.
        PartType::Message(message) => message.raw_message().to_vec(),
        PartType::Multipart(_) => Vec::new(),
    }
}

/// Parse a raw email byte slice into a `ParsedMessage`.
pub fn parse_raw_email(raw: &[u8]) -> Result<ParsedMessage> {
    let mut message = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| PebbleError::Sync("Failed to parse email message".to_string()))?;

    // Message-ID
    let message_id_header = message.message_id().map(|s| format!("<{s}>"));

    // In-Reply-To
    let in_reply_to = extract_id_list(message.in_reply_to());

    // References
    let references_header = extract_id_list(message.references());

    // Subject
    let subject = message.subject().unwrap_or("").to_string();

    // From
    let (from_address, from_name) = message
        .from()
        .and_then(|addr| addr.first())
        .map(|a| {
            (
                a.address().unwrap_or("").to_string(),
                a.name().unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();

    // To / Cc / Bcc
    let to_list = address_to_list(message.to());
    let cc_list = address_to_list(message.cc());
    let bcc_list = address_to_list(message.bcc());

    // Date
    let date = message.date().map(|d| d.to_timestamp()).unwrap_or(0);

    // Body
    let body_text = message
        .body_text(0)
        .map(|s| s.into_owned())
        .unwrap_or_default();
    let body_html = message
        .body_html(0)
        .map(|s| s.into_owned())
        .unwrap_or_default();

    // Snippet: prefer body_text, but strip HTML/CSS; fall back to body_html
    let snippet = build_snippet(&body_text, &body_html);

    // Attachments: collect metadata while the message is borrowed, then
    // move each body out of the parsed parts instead of copying it.
    let mut metas: Vec<(usize, AttachmentMeta)> = Vec::new();
    let mut total_bytes = 0usize;
    for (position, part) in message.attachments().enumerate() {
        if metas.len() >= MAX_ATTACHMENT_PARTS {
            tracing::warn!(
                "Message has more than {MAX_ATTACHMENT_PARTS} attachment parts; ignoring the rest"
            );
            break;
        }
        let size = part.len();
        if total_bytes.saturating_add(size) > MAX_TOTAL_ATTACHMENT_BYTES {
            tracing::warn!(
                "Skipping attachment part of {size} bytes: total attachment size would exceed {MAX_TOTAL_ATTACHMENT_BYTES} bytes"
            );
            continue;
        }
        total_bytes += size;
        let filename = part.attachment_name().unwrap_or("unnamed").to_string();
        let mime_type = part
            .content_type()
            .map(|ct| {
                if let Some(sub) = ct.subtype() {
                    format!("{}/{}", ct.ctype(), sub)
                } else {
                    ct.ctype().to_string()
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let content_id = part.content_id().map(|s| s.to_string());
        let is_inline = part
            .content_disposition()
            .map(|d| d.ctype() == "inline")
            .unwrap_or(false);
        let Some(&part_index) = message.attachments.get(position) else {
            continue;
        };
        metas.push((
            part_index,
            AttachmentMeta {
                filename,
                mime_type,
                size,
                content_id,
                is_inline,
            },
        ));
    }
    let mut attachments = Vec::with_capacity(metas.len());
    for (part_index, meta) in metas {
        let Some(part) = message.parts.get_mut(part_index) else {
            continue;
        };
        let body = std::mem::take(&mut part.body);
        attachments.push(AttachmentData {
            meta,
            data: part_body_into_bytes(body),
        });
    }

    let has_attachments = !attachments.is_empty();

    Ok(ParsedMessage {
        message_id_header,
        in_reply_to,
        references_header,
        subject,
        from_address,
        from_name,
        to_list,
        cc_list,
        bcc_list,
        date,
        body_text,
        body_html,
        snippet,
        has_attachments,
        attachments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unencoded_eml_attachment_keeps_only_the_nested_message() {
        let raw = b"From: Outer <outer@example.com>\r\n\
                    To: Bob <bob@example.com>\r\n\
                    Subject: Outer subject\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/mixed; boundary=\"b1\"\r\n\
                    \r\n\
                    --b1\r\n\
                    Content-Type: text/plain\r\n\
                    \r\n\
                    See attached.\r\n\
                    --b1\r\n\
                    Content-Type: message/rfc822\r\n\
                    Content-Disposition: attachment; filename=\"inner.eml\"\r\n\
                    \r\n\
                    From: Inner <inner@example.com>\r\n\
                    Subject: Inner subject\r\n\
                    \r\n\
                    Inner body\r\n\
                    --b1--\r\n";

        let parsed = parse_raw_email(raw).unwrap();
        let eml = parsed
            .attachments
            .iter()
            .find(|a| a.meta.mime_type.contains("rfc822") || a.meta.filename.ends_with(".eml"))
            .expect("the nested message is an attachment");
        let text = String::from_utf8_lossy(&eml.data);
        assert!(text.starts_with("From: Inner"), "{text}");
        assert!(text.contains("Inner body"), "{text}");
        assert!(!text.contains("Outer subject"), "{text}");
        assert_eq!(eml.meta.size, eml.data.len());
    }

    #[test]
    fn test_parse_simple_email() {
        let raw = b"From: Alice <alice@example.com>\r\n\
                    To: Bob <bob@example.com>\r\n\
                    Subject: Hello World\r\n\
                    Message-ID: <abc123@example.com>\r\n\
                    Date: Thu, 01 Jan 2015 00:00:00 +0000\r\n\
                    \r\n\
                    This is the body of the email.";

        let parsed = parse_raw_email(raw).unwrap();
        assert_eq!(parsed.subject, "Hello World");
        assert_eq!(parsed.from_address, "alice@example.com");
        assert_eq!(parsed.from_name, "Alice");
        assert_eq!(parsed.to_list.len(), 1);
        assert_eq!(parsed.to_list[0].address, "bob@example.com");
        assert_eq!(
            parsed.message_id_header.as_deref(),
            Some("<abc123@example.com>")
        );
        assert!(parsed.body_text.contains("body of the email"));
        assert!(!parsed.snippet.is_empty());
    }

    #[test]
    fn test_parse_html_email() {
        let raw = b"From: Sender <sender@example.com>\r\n\
                    To: Recipient <recv@example.com>\r\n\
                    Subject: HTML Email\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/alternative; boundary=\"boundary42\"\r\n\
                    \r\n\
                    --boundary42\r\n\
                    Content-Type: text/plain; charset=utf-8\r\n\
                    \r\n\
                    Plain text part\r\n\
                    --boundary42\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <p>HTML part</p>\r\n\
                    --boundary42--\r\n";

        let parsed = parse_raw_email(raw).unwrap();
        assert_eq!(parsed.subject, "HTML Email");
        assert!(!parsed.body_text.is_empty());
        assert!(!parsed.body_html.is_empty());
        assert!(parsed.body_html.contains("HTML part") || parsed.body_text.contains("Plain text"));
    }

    #[test]
    fn test_snippet_truncation() {
        // Build a body text longer than 200 chars
        let long_body = "word ".repeat(50); // 250 chars
        let raw = format!(
            "From: a@example.com\r\nTo: b@example.com\r\nSubject: Long\r\n\r\n{}",
            long_body
        );
        let parsed = parse_raw_email(raw.as_bytes()).unwrap();
        // Snippet should end with "..." and be <= 203 chars (200 + "...")
        assert!(
            parsed.snippet.ends_with("..."),
            "Expected snippet to end with '...', got: {:?}",
            parsed.snippet
        );
        // The part before "..." should be exactly 200 chars
        let without_ellipsis = parsed.snippet.trim_end_matches("...");
        assert_eq!(
            without_ellipsis.len(),
            200,
            "Expected 200 chars before '...', got {}",
            without_ellipsis.len()
        );
    }

    #[test]
    fn test_parse_with_in_reply_to() {
        let raw = b"From: Bob <bob@example.com>\r\n\
                    To: Alice <alice@example.com>\r\n\
                    Subject: Re: Hello World\r\n\
                    Message-ID: <reply123@example.com>\r\n\
                    In-Reply-To: <abc123@example.com>\r\n\
                    References: <abc123@example.com>\r\n\
                    Date: Thu, 01 Jan 2015 01:00:00 +0000\r\n\
                    \r\n\
                    Replying to your email.";

        let parsed = parse_raw_email(raw).unwrap();
        assert_eq!(
            parsed.message_id_header.as_deref(),
            Some("<reply123@example.com>")
        );
        assert_eq!(parsed.in_reply_to.as_deref(), Some("<abc123@example.com>"));
        assert_eq!(
            parsed.references_header.as_deref(),
            Some("<abc123@example.com>")
        );
    }

    #[test]
    fn test_parse_reply_references_match_stored_message_id_format() {
        let original = parse_raw_email(
            b"From: a@example.com\r\nMessage-ID: <root@example.com>\r\nSubject: Hi\r\n\r\nBody",
        )
        .unwrap();
        let reply = parse_raw_email(
            b"From: b@example.com\r\n\
              Message-ID: <reply@example.com>\r\n\
              In-Reply-To: <parent@example.com>\r\n\
              References: <root@example.com> <parent@example.com>\r\n\
              Subject: Re: Hi\r\n\r\nBody",
        )
        .unwrap();

        assert_eq!(
            original.message_id_header.as_deref(),
            Some("<root@example.com>")
        );
        assert_eq!(reply.in_reply_to.as_deref(), Some("<parent@example.com>"));
        assert_eq!(
            reply.references_header.as_deref(),
            Some("<root@example.com> <parent@example.com>")
        );
        assert!(reply
            .references_header
            .as_deref()
            .unwrap()
            .split_whitespace()
            .any(|id| Some(id) == original.message_id_header.as_deref()));
    }

    #[test]
    fn test_snippet_strips_css_from_html_only_email() {
        let raw = b"From: sender@example.com\r\n\
                    To: recv@example.com\r\n\
                    Subject: HTML Only\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <html><head><style>body, p { margin: 0; padding: 0; } .mail-container { max-width: 600px; }</style></head>\
                    <body><p>Your verification code is 123456</p></body></html>";

        let parsed = parse_raw_email(raw).unwrap();
        assert!(
            !parsed.snippet.contains("margin"),
            "Snippet should not contain CSS, got: {:?}",
            parsed.snippet
        );
        assert!(
            !parsed.snippet.contains("padding"),
            "Snippet should not contain CSS, got: {:?}",
            parsed.snippet
        );
        assert!(
            parsed.snippet.contains("verification code"),
            "Snippet should contain email body text, got: {:?}",
            parsed.snippet
        );
    }

    #[test]
    fn test_snippet_fallback_to_html_when_text_empty() {
        let raw = b"From: sender@example.com\r\n\
                    To: recv@example.com\r\n\
                    Subject: HTML Only No Text\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/alternative; boundary=\"boundary99\"\r\n\
                    \r\n\
                    --boundary99\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <html><body><p>Hello from HTML</p></body></html>\r\n\
                    --boundary99--\r\n";

        let parsed = parse_raw_email(raw).unwrap();
        assert!(
            parsed.snippet.contains("Hello from HTML"),
            "Snippet should fall back to HTML body, got: {:?}",
            parsed.snippet
        );
    }

    #[test]
    fn test_strip_html_for_snippet() {
        let html = r#"<html><head><style>body { margin: 0; }</style></head><body><p>Hello World</p></body></html>"#;
        let result = pebble_core::strip_html_for_snippet(html);
        assert!(!result.contains("margin"));
        assert!(result.contains("Hello World"));
    }

    fn multipart_with_attachments(count: usize) -> Vec<u8> {
        let mut raw = String::from(
            "From: a@example.com\r\nSubject: files\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nbody\r\n",
        );
        for index in 0..count {
            raw.push_str(&format!(
                "--b\r\nContent-Type: application/octet-stream\r\n\
                 Content-Disposition: attachment; filename=\"f{index}.bin\"\r\n\
                 Content-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n"
            ));
        }
        raw.push_str("--b--\r\n");
        raw.into_bytes()
    }

    #[test]
    fn attachments_are_decoded_and_moved_out_of_the_parse_tree() {
        let parsed = parse_raw_email(&multipart_with_attachments(2)).unwrap();
        assert!(parsed.has_attachments);
        assert_eq!(parsed.attachments.len(), 2);
        assert_eq!(parsed.attachments[0].meta.filename, "f0.bin");
        assert_eq!(parsed.attachments[1].meta.filename, "f1.bin");
        assert_eq!(parsed.attachments[0].data, b"hello");
        assert_eq!(parsed.attachments[0].meta.size, 5);
        assert_eq!(parsed.body_text.trim(), "body");
    }

    #[test]
    fn attachment_part_count_is_capped() {
        let parsed =
            parse_raw_email(&multipart_with_attachments(MAX_ATTACHMENT_PARTS + 3)).unwrap();
        assert_eq!(parsed.attachments.len(), MAX_ATTACHMENT_PARTS);
    }
}
