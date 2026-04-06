use crate::imap::ConnectionSecurity;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Body, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use pebble_core::{PebbleError, Result};
use std::path::Path;

pub struct SmtpSender {
    host: String,
    port: u16,
    credentials: Credentials,
    security: ConnectionSecurity,
}

impl SmtpSender {
    pub fn new(host: String, port: u16, username: String, password: String, security: ConnectionSecurity) -> Self {
        Self {
            host,
            port,
            credentials: Credentials::new(username, password),
            security,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &self,
        from: &str,
        to: &[String],
        cc: &[String],
        bcc: &[String],
        subject: &str,
        body_text: &str,
        body_html: Option<&str>,
        in_reply_to: Option<&str>,
        attachment_paths: &[String],
    ) -> Result<()> {
        if to.is_empty() {
            return Err(PebbleError::Internal("No recipients".to_string()));
        }

        let from_mailbox: Mailbox = from
            .parse()
            .map_err(|e| PebbleError::Internal(format!("Invalid from address: {e}")))?;

        let mut builder = lettre::Message::builder()
            .from(from_mailbox)
            .subject(subject);

        for addr in to {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e| PebbleError::Internal(format!("Invalid to address '{addr}': {e}")))?;
            builder = builder.to(mailbox);
        }

        for addr in cc {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e| PebbleError::Internal(format!("Invalid cc address '{addr}': {e}")))?;
            builder = builder.cc(mailbox);
        }

        for addr in bcc {
            let mailbox: Mailbox = addr
                .parse()
                .map_err(|e| PebbleError::Internal(format!("Invalid bcc address '{addr}': {e}")))?;
            builder = builder.bcc(mailbox);
        }

        if let Some(reply_to) = in_reply_to {
            builder = builder.in_reply_to(reply_to.to_string());
        }

        let alternative_body = MultiPart::alternative()
            .singlepart(
                SinglePart::builder()
                    .content_type(ContentType::TEXT_PLAIN)
                    .body(body_text.to_string()),
            )
            .singlepart(
                SinglePart::builder()
                    .content_type(ContentType::TEXT_HTML)
                    .body(
                        body_html
                            .unwrap_or(body_text)
                            .to_string(),
                    ),
            );

        let email = if attachment_paths.is_empty() {
            builder
                .multipart(alternative_body)
                .map_err(|e| PebbleError::Internal(format!("Failed to build email: {e}")))?
        } else {
            let mut mixed = MultiPart::mixed().multipart(alternative_body);

            for path_str in attachment_paths {
                let path = Path::new(path_str);
                let filename = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment")
                    .to_string();

                let file_bytes = std::fs::read(path).map_err(|e| {
                    PebbleError::Internal(format!(
                        "Failed to read attachment '{}': {e}",
                        path_str
                    ))
                })?;

                let content_type = mime_type_from_extension(
                    path.extension().and_then(|e| e.to_str()).unwrap_or(""),
                );

                let attachment = Attachment::new(filename)
                    .body(Body::new(file_bytes), content_type);

                mixed = mixed.singlepart(attachment);
            }

            builder
                .multipart(mixed)
                .map_err(|e| PebbleError::Internal(format!("Failed to build email: {e}")))?
        };

        let transport = match self.security {
            ConnectionSecurity::Tls => {
                SmtpTransport::relay(&self.host)
                    .map_err(|e| PebbleError::Network(format!("SMTP relay error: {e}")))?
                    .port(self.port)
                    .credentials(self.credentials.clone())
                    .build()
            }
            ConnectionSecurity::StartTls => {
                SmtpTransport::starttls_relay(&self.host)
                    .map_err(|e| PebbleError::Network(format!("SMTP STARTTLS error: {e}")))?
                    .port(self.port)
                    .credentials(self.credentials.clone())
                    .build()
            }
            ConnectionSecurity::Plain => {
                SmtpTransport::builder_dangerous(&self.host)
                    .port(self.port)
                    .credentials(self.credentials.clone())
                    .build()
            }
        };

        transport
            .send(&email)
            .map_err(|e| PebbleError::Network(format!("SMTP send failed: {e}")))?;

        Ok(())
    }
}

/// Map common file extensions to MIME content types.
fn mime_type_from_extension(ext: &str) -> ContentType {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => ContentType::parse("application/pdf").unwrap(),
        "zip" => ContentType::parse("application/zip").unwrap(),
        "gz" | "gzip" => ContentType::parse("application/gzip").unwrap(),
        "doc" => ContentType::parse("application/msword").unwrap(),
        "docx" => ContentType::parse(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )
        .unwrap(),
        "xls" => ContentType::parse("application/vnd.ms-excel").unwrap(),
        "xlsx" => ContentType::parse(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .unwrap(),
        "ppt" => ContentType::parse("application/vnd.ms-powerpoint").unwrap(),
        "pptx" => ContentType::parse(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        )
        .unwrap(),
        "png" => ContentType::parse("image/png").unwrap(),
        "jpg" | "jpeg" => ContentType::parse("image/jpeg").unwrap(),
        "gif" => ContentType::parse("image/gif").unwrap(),
        "svg" => ContentType::parse("image/svg+xml").unwrap(),
        "webp" => ContentType::parse("image/webp").unwrap(),
        "mp3" => ContentType::parse("audio/mpeg").unwrap(),
        "mp4" => ContentType::parse("video/mp4").unwrap(),
        "txt" => ContentType::TEXT_PLAIN,
        "html" | "htm" => ContentType::TEXT_HTML,
        "csv" => ContentType::parse("text/csv").unwrap(),
        "json" => ContentType::parse("application/json").unwrap(),
        "xml" => ContentType::parse("application/xml").unwrap(),
        "eml" => ContentType::parse("message/rfc822").unwrap(),
        _ => ContentType::parse("application/octet-stream").unwrap(),
    }
}
