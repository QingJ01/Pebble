pub mod folder_management;

mod connect;
mod fetch;
mod ops;
mod transport;

pub(crate) use transport::AcceptInvalidCertsVerifier;
use transport::*;

pub fn decode_folder_name(remote_id: &str) -> String {
    utf7_imap::decode_utf7_imap(remote_id.to_owned())
}

use std::collections::HashSet;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_imap::{types::NameAttribute, Client};
use futures::TryStreamExt;
use pebble_core::{new_id, Folder, FolderRole, FolderType, PebbleError, Result};
use serde::de::Deserializer;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_native_tls as async_native_tls;
use tokio_rustls::client::TlsStream;
use tracing::debug;

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::ParsedCertificate,
    DigitallySignedStruct, Error as TlsError, SignatureScheme,
};

/// Connection security mode for mail protocols.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionSecurity {
    /// Implicit TLS — connect over TLS immediately (IMAP 993, SMTP 465).
    #[default]
    Tls,
    /// STARTTLS — connect plain then upgrade to TLS (IMAP 143, SMTP 587).
    #[serde(rename = "starttls")]
    StartTls,
    /// No encryption (not recommended).
    Plain,
}

/// Optional SOCKS5 proxy configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
}

/// Configuration for an IMAP connection.
#[derive(Clone, serde::Serialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub security: ConnectionSecurity,
    #[serde(default, skip_serializing_if = "is_false")]
    pub accept_invalid_certs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

impl std::fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("security", &self.security)
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .field("proxy", &self.proxy)
            .finish()
    }
}

// Custom Deserialize to handle legacy `use_tls: bool` configs.
impl<'de> serde::Deserialize<'de> for ImapConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: String,
            #[serde(default)]
            security: Option<ConnectionSecurity>,
            #[serde(default)]
            use_tls: Option<bool>,
            #[serde(default)]
            accept_invalid_certs: bool,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or(match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(ImapConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            accept_invalid_certs: raw.accept_invalid_certs,
            proxy: raw.proxy,
        })
    }
}

/// Configuration for an SMTP connection.
#[derive(Clone, serde::Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub security: ConnectionSecurity,
    #[serde(default, skip_serializing_if = "is_false")]
    pub accept_invalid_certs: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("security", &self.security)
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .field("proxy", &self.proxy)
            .finish()
    }
}

// Custom Deserialize to handle legacy `use_tls: bool` configs.
impl<'de> serde::Deserialize<'de> for SmtpConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: String,
            #[serde(default)]
            security: Option<ConnectionSecurity>,
            #[serde(default)]
            use_tls: Option<bool>,
            #[serde(default)]
            accept_invalid_certs: bool,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or(match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(SmtpConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            accept_invalid_certs: raw.accept_invalid_certs,
            proxy: raw.proxy,
        })
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn incremental_fetch_uids(since_uid: Option<u32>, searched_uids: &[u32]) -> Vec<u32> {
    let Some(since_uid) = since_uid else {
        return searched_uids.to_vec();
    };
    let mut uids = searched_uids
        .iter()
        .copied()
        .filter(|uid| *uid > since_uid)
        .collect::<Vec<_>>();
    uids.sort_unstable();
    uids.dedup();
    uids
}

fn should_search_incremental_uids(next_uid: u32, mailbox_uid_next: Option<u32>) -> bool {
    mailbox_uid_next.is_none_or(|uid_next| next_uid < uid_next)
}

/// Longest UID-set argument placed on one command line. Many servers reject
/// command lines above ~8KB, so stay comfortably below that.
pub(crate) const MAX_UID_SET_CHARS: usize = 7_000;

/// Messages larger than this are never downloaded in full; only their
/// headers are kept and the body is reported as too large.
pub const MAX_IMAP_MESSAGE_BYTES: u32 = 50 * 1024 * 1024;

/// Upper bounds for one `UID FETCH ... BODY.PEEK[]` round trip so a single
/// batch never holds an unbounded amount of message data in memory.
pub(crate) const IMAP_BODY_BATCH_MAX_MESSAGES: usize = 50;
pub(crate) const IMAP_BODY_BATCH_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Compress UIDs into IMAP sequence-set syntax (`1:5,7,9:12`), split into
/// chunks whose textual length never exceeds [`MAX_UID_SET_CHARS`].
/// Input order and duplicates do not matter; UID 0 (invalid) is dropped.
pub fn compress_uid_set(uids: &[u32]) -> Vec<String> {
    compress_uid_set_with_limit(uids, MAX_UID_SET_CHARS)
}

fn compress_uid_set_with_limit(uids: &[u32], max_chars: usize) -> Vec<String> {
    let mut sorted: Vec<u32> = uids.iter().copied().filter(|uid| *uid > 0).collect();
    sorted.sort_unstable();
    sorted.dedup();

    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && end.checked_add(1) == Some(sorted[index + 1]) {
            index += 1;
            end = sorted[index];
        }
        index += 1;
        let token = if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        };
        if !current.is_empty() && current.len() + 1 + token.len() > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(&token);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// How a set of messages should be downloaded given their RFC822.SIZE.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImapBodyFetchPlan {
    /// UID batches to fetch with `BODY.PEEK[]`, each bounded by count and bytes.
    pub batches: Vec<Vec<u32>>,
    /// `(uid, size)` of messages too large to download in full.
    pub oversized: Vec<(u32, u32)>,
}

/// Split `(uid, size)` pairs into bounded body-fetch batches, setting aside
/// messages above `max_message_bytes`. Input order is preserved.
pub fn plan_imap_body_batches(
    sizes: &[(u32, u32)],
    max_message_bytes: u32,
    max_batch_messages: usize,
    max_batch_bytes: u64,
) -> ImapBodyFetchPlan {
    let max_batch_messages = max_batch_messages.max(1);
    let mut plan = ImapBodyFetchPlan::default();
    let mut batch = Vec::new();
    let mut batch_bytes = 0u64;
    for &(uid, size) in sizes {
        if size > max_message_bytes {
            plan.oversized.push((uid, size));
            continue;
        }
        let size = u64::from(size);
        if !batch.is_empty()
            && (batch.len() >= max_batch_messages || batch_bytes + size > max_batch_bytes)
        {
            plan.batches.push(std::mem::take(&mut batch));
            batch_bytes = 0;
        }
        batch.push(uid);
        batch_bytes += size;
    }
    if !batch.is_empty() {
        plan.batches.push(batch);
    }
    plan
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImapMailboxStatus {
    pub uid_validity: Option<u32>,
    pub highest_modseq: Option<u64>,
    pub uid_next: Option<u32>,
    /// Number of messages in the mailbox as reported by SELECT.
    pub exists: u32,
}

fn imap_since_date(timestamp: i64) -> String {
    let (year, month, day) = pebble_core::utc_date_from_unix_seconds(timestamp);
    let month = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    };
    format!("{day:02}-{month}-{year:04}")
}

fn imap_since_search_criteria(cutoff: i64, since_uid: Option<u32>) -> Option<String> {
    let mut criteria = format!("SINCE {}", imap_since_date(cutoff));
    if let Some(uid) = since_uid {
        let next_uid = uid.checked_add(1)?;
        criteria.push_str(&format!(" UID {next_uid}:*"));
    }
    Some(criteria)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UidSection {
    Body,
    Header,
}

/// An IMAP provider that manages a connection and session.
pub struct ImapProvider {
    config: ImapConfig,
    session: Arc<Mutex<Option<ImapSession>>>,
}

impl ImapProvider {}

/// Parse flags from an iterator of `Flag` values.
fn parse_flags<'a>(flags: impl Iterator<Item = async_imap::types::Flag<'a>>) -> (bool, bool) {
    let mut is_read = false;
    let mut is_starred = false;
    for flag in flags {
        match flag {
            async_imap::types::Flag::Seen => is_read = true,
            async_imap::types::Flag::Flagged => is_starred = true,
            _ => {}
        }
    }
    (is_read, is_starred)
}

fn should_sync_listed_mailbox(attributes: &[NameAttribute<'_>]) -> bool {
    !attributes
        .iter()
        .any(|attribute| matches!(attribute, NameAttribute::NoSelect))
}

fn detect_folder_role_from_attributes(attributes: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attributes.iter().find_map(|attribute| match attribute {
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Junk => Some(FolderRole::Spam),
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Extension(value) if value.eq_ignore_ascii_case("\\Inbox") => {
            Some(FolderRole::Inbox)
        }
        _ => None,
    })
}

/// Detect a folder role based on its name.
pub fn detect_folder_role(name: &str) -> Option<FolderRole> {
    let lower = name.to_lowercase();
    // Check last component after hierarchy separator
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    let leaf = leaf.rsplit('.').next().unwrap_or(leaf);

    if leaf == "inbox" || leaf == "收件箱" {
        Some(FolderRole::Inbox)
    } else if leaf.contains("sent") || leaf.contains("已发送") || leaf.contains("已发件") {
        Some(FolderRole::Sent)
    } else if leaf.contains("draft") || leaf.contains("草稿") {
        Some(FolderRole::Drafts)
    } else if leaf.contains("trash")
        || leaf.contains("deleted")
        || leaf.contains("已删除")
        || leaf.contains("废纸篓")
    {
        Some(FolderRole::Trash)
    } else if leaf.contains("archive") || leaf.contains("归档") || leaf.contains("存档") {
        Some(FolderRole::Archive)
    } else if leaf.contains("spam")
        || leaf.contains("junk")
        || leaf.contains("垃圾")
        || leaf.contains("病毒")
        || leaf.contains("广告")
    {
        Some(FolderRole::Spam)
    } else {
        None
    }
}

/// Sort order for folder roles.
pub fn folder_sort_order(role: &Option<FolderRole>) -> i32 {
    match role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Drafts) => 1,
        Some(FolderRole::Sent) => 2,
        Some(FolderRole::Archive) => 3,
        Some(FolderRole::Spam) => 4,
        Some(FolderRole::Trash) => 5,
        None => 100,
    }
}

#[cfg(test)]
mod tls_config_tests {
    use super::{
        build_native_tls_connector, build_tls_connector, imap_timeout_error, ImapConfig, SmtpConfig,
    };

    #[test]
    fn build_tls_connector_returns_result() {
        assert!(build_tls_connector(false).is_ok());
        assert!(build_tls_connector(true).is_ok());
    }

    #[test]
    fn build_native_tls_connector_returns_result() {
        assert!(build_native_tls_connector(false).is_ok());
        assert!(build_native_tls_connector(true).is_ok());
    }

    #[test]
    fn imap_config_defaults_to_certificate_verification() {
        let config: ImapConfig = serde_json::from_value(serde_json::json!({
            "host": "mail.example.com",
            "port": 993,
            "username": "user",
            "password": "secret",
            "security": "tls"
        }))
        .unwrap();

        assert!(!config.accept_invalid_certs);
    }

    #[test]
    fn smtp_config_preserves_invalid_certificate_override() {
        let config: SmtpConfig = serde_json::from_value(serde_json::json!({
            "host": "mail.example.com",
            "port": 465,
            "username": "user",
            "password": "secret",
            "security": "tls",
            "accept_invalid_certs": true
        }))
        .unwrap();

        assert!(config.accept_invalid_certs);
    }

    #[test]
    fn imap_timeout_error_names_operation_and_seconds() {
        let error = imap_timeout_error("UID FETCH", 30);

        assert_eq!(
            error.to_string(),
            "Network error: UID FETCH timed out after 30s"
        );
    }
}

#[cfg(test)]
mod folder_list_tests {
    use async_imap::types::NameAttribute;
    use pebble_core::FolderRole;

    use super::{detect_folder_role_from_attributes, should_sync_listed_mailbox};

    #[test]
    fn special_use_attributes_detect_system_folder_roles() {
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Sent]),
            Some(FolderRole::Sent)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Trash]),
            Some(FolderRole::Trash)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Drafts]),
            Some(FolderRole::Drafts)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Archive]),
            Some(FolderRole::Archive)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Junk]),
            Some(FolderRole::Spam)
        );
    }

    #[test]
    fn extension_inbox_attribute_detects_inbox_role() {
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Extension("\\Inbox".into())]),
            Some(FolderRole::Inbox)
        );
    }

    #[test]
    fn noselect_mailboxes_are_not_synced_as_real_folders() {
        assert!(!should_sync_listed_mailbox(&[NameAttribute::NoSelect]));
        assert!(!should_sync_listed_mailbox(&[
            NameAttribute::NoSelect,
            NameAttribute::Extension("\\HasChildren".into()),
        ]));
        assert!(should_sync_listed_mailbox(&[NameAttribute::Sent]));
    }
}

#[cfg(test)]
mod incremental_uid_tests {
    use super::{
        compress_uid_set, compress_uid_set_with_limit, imap_since_search_criteria,
        incremental_fetch_uids, plan_imap_body_batches, should_search_incremental_uids,
        starttls_session_prefix, ImapBodyFetchPlan, MAX_UID_SET_CHARS,
    };

    #[test]
    fn sync_range_search_uses_inclusive_utc_date_and_uid_cursor() {
        assert_eq!(
            imap_since_search_criteria(1_704_067_200, Some(42)),
            Some("SINCE 01-Jan-2024 UID 43:*".to_string())
        );
        assert_eq!(
            imap_since_search_criteria(1_704_067_200, Some(u32::MAX)),
            None
        );
    }

    #[test]
    fn uid_set_compresses_contiguous_runs_into_ranges() {
        assert_eq!(compress_uid_set(&[]), Vec::<String>::new());
        assert_eq!(compress_uid_set(&[7]), vec!["7".to_string()]);
        assert_eq!(
            compress_uid_set(&[5, 3, 4, 1, 9, 10, 11, 4, 0]),
            vec!["1,3:5,9:11".to_string()]
        );
        assert_eq!(
            compress_uid_set(&[u32::MAX - 1, u32::MAX]),
            vec![format!("{}:{}", u32::MAX - 1, u32::MAX)]
        );
    }

    #[test]
    fn uid_set_of_100k_contiguous_uids_is_one_short_range() {
        let uids: Vec<u32> = (1..=100_000).collect();
        assert_eq!(compress_uid_set(&uids), vec!["1:100000".to_string()]);
    }

    #[test]
    fn uid_set_chunks_stay_below_the_line_limit_and_cover_every_uid() {
        // Every other UID: no ranges possible, forcing many chunks.
        let uids: Vec<u32> = (1..=200_000).step_by(2).collect();
        let chunks = compress_uid_set(&uids);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|chunk| chunk.len() <= MAX_UID_SET_CHARS));
        let mut covered = Vec::new();
        for chunk in &chunks {
            for token in chunk.split(',') {
                covered.push(token.parse::<u32>().unwrap());
            }
        }
        assert_eq!(covered, uids);

        let small = compress_uid_set_with_limit(&[1, 3, 5, 7], 3);
        assert_eq!(small, vec!["1,3", "5,7"]);
    }

    #[test]
    fn body_batches_are_bounded_and_oversized_messages_are_set_aside() {
        let sizes = [(1, 10), (2, 10), (3, 500), (4, 10), (5, 10), (6, 60)];
        let plan = plan_imap_body_batches(&sizes, 100, 2, 25);
        assert_eq!(plan.oversized, vec![(3, 500)]);
        assert_eq!(plan.batches, vec![vec![1, 2], vec![4, 5], vec![6]]);

        let plan = plan_imap_body_batches(&[], 100, 2, 25);
        assert_eq!(plan, ImapBodyFetchPlan::default());
    }

    #[test]
    fn starttls_never_replays_pre_tls_bytes() {
        assert!(starttls_session_prefix().is_empty());
    }

    #[test]
    fn incremental_fetch_skips_when_no_uid_is_newer_than_cursor() {
        let uids = incremental_fetch_uids(Some(3291), &[1, 42, 3291]);

        assert!(uids.is_empty());
    }

    #[test]
    fn incremental_fetch_returns_only_newer_uids_sorted_and_deduped() {
        let uids = incremental_fetch_uids(Some(42), &[45, 41, 43, 45, 42]);

        assert_eq!(uids, vec![43, 45]);
    }

    #[test]
    fn incremental_fetch_skips_search_when_uidnext_shows_no_new_mail() {
        assert!(!should_search_incremental_uids(3292, Some(3292)));
        assert!(!should_search_incremental_uids(3293, Some(3292)));
    }

    #[test]
    fn incremental_fetch_searches_when_uidnext_is_missing_or_newer() {
        assert!(should_search_incremental_uids(3292, None));
        assert!(should_search_incremental_uids(3292, Some(3293)));
    }
}
