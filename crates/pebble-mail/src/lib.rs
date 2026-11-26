pub(crate) mod backoff;
pub mod gmail_sync;
pub mod idle;
pub mod imap;
pub mod outlook_sync;
pub mod parser;
pub mod pop3;
pub mod pop3_sync;
pub mod provider;
pub mod realtime_policy;
pub mod reconcile;
pub mod sender;
pub mod smtp;
pub mod sync;
pub mod thread;

pub use gmail_sync::GmailSyncWorker;
pub use imap::{ConnectionSecurity, ImapConfig, ImapProvider, ProxyConfig, SmtpConfig};
pub use outlook_sync::OutlookSyncWorker;
pub use pop3::{Pop3Config, Pop3MessageRef, Pop3Provider};
pub use pop3_sync::Pop3SyncWorker;
pub use provider::gmail::GmailProvider;
pub use provider::imap_provider::ImapMailProvider;
pub use provider::outlook::{should_hide_outlook_folder, OutlookProvider};
pub use provider::pop3_provider::Pop3MailProvider;
pub use realtime_policy::{RealtimeContext, RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
pub use sync::{
    StoredMessage, SyncConfig, SyncError, SyncProgress, SyncRuntimeStatus, SyncWorker,
    ThreadRefresh,
};
