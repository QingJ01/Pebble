use pebble_core::{PebbleError, Result};
use pebble_mail::parser::{parse_raw_email, ParsedMessage};
use pebble_store::mail_archive::{ArchiveAttachment, ArchiveMetadata};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Read, Write};

pub const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_ATTACHMENTS_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_MESSAGES: usize = 10_000;
pub const MAX_FILES: usize = 64;

pub fn invalid(message: &str) -> PebbleError {
    PebbleError::Validation(message.into())
}

pub fn io_error(error: std::io::Error) -> PebbleError {
    // Never log message contents or local source paths.
    PebbleError::Storage(format!("Archive file I/O failed: {}", error.kind()))
}

pub fn fingerprint(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

pub fn parse(raw: &[u8]) -> Result<(ArchiveMetadata, ParsedMessage)> {
    if raw.is_empty() || raw.len() > MAX_MESSAGE_BYTES || raw.contains(&0) {
        return Err(invalid("Empty, binary or oversized MIME message"));
    }
    let header_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .or_else(|| raw.windows(2).position(|w| w == b"\n\n"))
        .ok_or_else(|| invalid("MIME message has no header/body separator"))?;
    if header_end > 256 * 1024 || !raw[..header_end].contains(&b':') {
        return Err(invalid("Invalid or oversized MIME headers"));
    }
    // Bound MIME part/header fan-out before invoking the general MIME parser.
    if raw
        .split(|b| *b == b'\n')
        .filter(|line| line.starts_with(b"--"))
        .take(2050)
        .count()
        > 2048
    {
        return Err(invalid("Too many MIME boundaries"));
    }
    let parsed = parse_raw_email(raw)?;
    if parsed.body_text.len() > MAX_BODY_BYTES
        || parsed.body_html.len() > MAX_BODY_BYTES
        || parsed.attachments.len() > 256
    {
        return Err(invalid(
            "MIME body or attachment count exceeds archive limits",
        ));
    }
    let total: usize = parsed.attachments.iter().map(|a| a.data.len()).sum();
    if total > MAX_ATTACHMENTS_BYTES
        || parsed
            .attachments
            .iter()
            .any(|a| a.data.len() > MAX_ATTACHMENT_BYTES)
    {
        return Err(invalid("Decoded attachments exceed archive limits"));
    }
    let metadata = ArchiveMetadata {
        message_id_header: parsed.message_id_header.clone(),
        subject: parsed.subject.clone(),
        from_address: parsed.from_address.clone(),
        from_name: parsed.from_name.clone(),
        to_list: parsed.to_list.clone(),
        cc_list: parsed.cc_list.clone(),
        bcc_list: parsed.bcc_list.clone(),
        date: parsed.date,
        body_text: parsed.body_text.clone(),
        has_html: !parsed.body_html.is_empty(),
        attachments: parsed
            .attachments
            .iter()
            .map(|a| ArchiveAttachment {
                filename: a.meta.filename.clone(),
                mime_type: a.meta.mime_type.clone(),
                size: a.data.len(),
                content_id: a.meta.content_id.clone(),
                is_inline: a.meta.is_inline,
                sha256: fingerprint(&a.data),
            })
            .collect(),
    };
    Ok((metadata, parsed))
}

pub fn read_eml(reader: impl Read) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    reader
        .take(MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(io_error)?;
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(invalid("EML exceeds 32 MiB"));
    }
    Ok(raw)
}

fn bounded_line(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let buffer = reader.fill_buf().map_err(io_error)?;
        if buffer.is_empty() {
            break;
        }
        let len = buffer
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(buffer.len());
        if line.len() + len > MAX_MESSAGE_BYTES {
            return Err(invalid("MBOX line exceeds 32 MiB"));
        }
        let done = buffer[len - 1] == b'\n';
        line.extend_from_slice(&buffer[..len]);
        reader.consume(len);
        if done {
            break;
        }
    }
    Ok(line)
}

/// Streaming mboxrd reader. Unescaped From lines delimit messages; one quoting
/// level is removed only from ^>+From lines. Each record remains independently
/// bounded and the callback may stop at any committed-message boundary.
pub fn read_mbox(
    mut reader: impl BufRead,
    mut accept: impl FnMut(Vec<u8>) -> Result<bool>,
) -> Result<()> {
    let mut raw = Vec::new();
    let mut started = false;
    loop {
        let line = bounded_line(&mut reader)?;
        if line.is_empty() {
            break;
        }
        if line.starts_with(b"From ") {
            if started {
                strip_framing_newline(&mut raw);
                if !accept(std::mem::take(&mut raw))? {
                    return Ok(());
                }
            }
            started = true;
        } else {
            if !started {
                return Err(invalid("MBOX must begin with a From envelope line"));
            }
            let quotes = line.iter().take_while(|b| **b == b'>').count();
            let content = if quotes > 0 && line[quotes..].starts_with(b"From ") {
                &line[1..]
            } else {
                &line
            };
            if raw.len() + content.len() > MAX_MESSAGE_BYTES {
                return Err(invalid("MBOX message exceeds 32 MiB"));
            }
            raw.extend_from_slice(content);
        }
    }
    if !started {
        return Err(invalid("MBOX is empty"));
    }
    strip_framing_newline(&mut raw);
    accept(raw)?;
    Ok(())
}

fn strip_framing_newline(raw: &mut Vec<u8>) {
    // A blank line separates mbox records; don't remove the MIME's own EOL.
    if raw.ends_with(b"\r\n\r\n") {
        raw.truncate(raw.len() - 2);
    } else if raw.ends_with(b"\n\n") || raw.ends_with(b"\r\n\n") {
        raw.pop();
    }
}

pub fn write_mbox(writer: &mut impl Write, raw: &[u8]) -> Result<()> {
    writer
        .write_all(b"From MAILER-DAEMON Thu Jan  1 00:00:00 1970\n")
        .map_err(io_error)?;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let quotes = line.iter().take_while(|b| **b == b'>').count();
        if line[quotes..].starts_with(b"From ") {
            writer.write_all(b">").map_err(io_error)?;
        }
        writer.write_all(line).map_err(io_error)?;
    }
    if !raw.ends_with(b"\n") {
        writer.write_all(b"\n").map_err(io_error)?;
    }
    writer.write_all(b"\n").map_err(io_error)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub const MIME: &[u8] = b"From: =?UTF-8?B?5rWL6K+V?= <a@example.com>\r\nTo: A <to@example.com>\r\nCc: cc@example.com\r\nBcc: hidden@example.com\r\nSubject: =?UTF-8?B?5L2g5aW9?=\r\nDate: Thu, 01 Jan 2015 00:00:00 +0200\r\nMessage-ID: <same@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n--a\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nUnicode body \xe4\xbd\xa0\xe5\xa5\xbd\r\nFrom body\r\n>From quoted\r\n>>From nested\r\n--a\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Unicode</p><img src=\"cid:picture\"><script>evil()</script>\r\n--a--\r\n--m\r\nContent-Type: image/png\r\nContent-Disposition: inline; filename=\"image.png\"\r\nContent-ID: <picture>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw==\r\n--m\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"report.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC/w==\r\n--m--\r\n";

    #[test]
    fn unicode_recipients_date_mime_and_attachment_hashes_are_preserved() {
        let (meta, parsed) = parse(MIME).unwrap();
        assert_eq!(meta.subject, "你好");
        assert_eq!(meta.from_name, "测试");
        assert_eq!(meta.to_list[0].address, "to@example.com");
        assert_eq!(meta.cc_list[0].address, "cc@example.com");
        assert_eq!(meta.bcc_list[0].address, "hidden@example.com");
        assert_eq!(meta.date, 1420063200);
        assert!(meta.body_text.contains("你好"));
        assert!(parsed.body_html.contains("cid:picture"));
        assert_eq!(meta.attachments.len(), 2);
        assert!(meta.attachments[0].is_inline);
        assert_eq!(meta.attachments[0].content_id.as_deref(), Some("picture"));
        assert_eq!(meta.attachments[1].sha256, fingerprint(&[0, 1, 2, 255]));
    }

    #[test]
    fn eml_is_byte_exact_and_mboxrd_round_trips_from_escaping() {
        assert_eq!(read_eml(MIME).unwrap(), MIME);
        let mut mbox = Vec::new();
        write_mbox(&mut mbox, MIME).unwrap();
        write_mbox(&mut mbox, MIME).unwrap();
        let mut messages = Vec::new();
        read_mbox(std::io::Cursor::new(mbox), |raw| {
            messages.push(raw);
            Ok(true)
        })
        .unwrap();
        assert_eq!(messages, vec![MIME.to_vec(), MIME.to_vec()]);
        let mut crlf = b"From sender Thu Jan  1 00:00:00 1970\r\n".to_vec();
        crlf.extend_from_slice(b"Subject: CRLF\r\n\r\nbody\r\n\r\n");
        let mut record = Vec::new();
        read_mbox(std::io::Cursor::new(crlf), |raw| {
            record = raw;
            Ok(true)
        })
        .unwrap();
        assert_eq!(record, b"Subject: CRLF\r\n\r\nbody\r\n");
    }

    #[test]
    fn malformed_binary_and_oversized_inputs_fail_closed() {
        assert!(parse(b"not email").is_err());
        assert!(parse(b"Subject: hello\n\n\0").is_err());
        assert!(read_eml(std::io::repeat(b'x').take(MAX_MESSAGE_BYTES as u64 + 1)).is_err());
        assert!(read_mbox(std::io::Cursor::new(b"no envelope\n"), |_| Ok(true)).is_err());
        let raw = [
            b"Subject: huge\n\n".as_slice(),
            &vec![b'a'; MAX_BODY_BYTES + 1],
        ]
        .concat();
        assert!(parse(&raw).is_err());
    }

    #[test]
    fn missing_or_duplicate_message_ids_do_not_determine_content_identity() {
        let (_, parsed) = parse(b"Subject: one\n\nbody").unwrap();
        assert_eq!(parsed.message_id_header, None);
        let a = b"Message-ID: <same>\n\nfirst";
        let b = b"Message-ID: <same>\n\nsecond";
        assert_ne!(fingerprint(a), fingerprint(b));
        assert_eq!(fingerprint(a), fingerprint(a));
    }
}
