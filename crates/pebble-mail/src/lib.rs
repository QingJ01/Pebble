pub mod gmail_sync;
pub mod idle;
pub mod imap;
pub mod outlook_sync;
pub mod parser;
pub mod provider;
pub mod reconcile;
pub mod smtp;
pub mod sync;
pub mod thread;

pub use gmail_sync::GmailSyncWorker;
pub use imap::{ConnectionSecurity, ImapConfig, ImapProvider, ProxyConfig, SmtpConfig};
pub use outlook_sync::OutlookSyncWorker;
pub use provider::gmail::GmailProvider;
pub use provider::imap_provider::ImapMailProvider;
pub use provider::outlook::OutlookProvider;
pub use sync::{StoredMessage, SyncConfig, SyncError, SyncWorker};
