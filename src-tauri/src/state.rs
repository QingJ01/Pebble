use crate::realtime::SyncTrigger;
use pebble_crypto::CryptoService;
use pebble_search::TantivySearch;
use pebble_store::Store;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

pub type KeyedLockRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>;
pub type OAuthAccountLockRegistry = KeyedLockRegistry;

pub struct SyncHandle {
    pub stop_tx: watch::Sender<bool>,
    pub trigger_tx: mpsc::UnboundedSender<SyncTrigger>,
    pub task: tokio::task::JoinHandle<()>,
}

pub struct AppState {
    pub store: Arc<Store>,
    pub search: Arc<TantivySearch>,
    pub crypto: Arc<CryptoService>,
    pub oauth_account_locks: OAuthAccountLockRegistry,
    pub secure_user_data_locks: KeyedLockRegistry,
    pub sync_handles: Mutex<HashMap<String, SyncHandle>>,
    /// Kept alive so the snooze watcher's `stop_rx` remains open.
    #[allow(dead_code)]
    pub snooze_stop_tx: std::sync::mpsc::Sender<()>,
    pub attachments_dir: PathBuf,
    pub notifications_enabled: Arc<AtomicBool>,
    pub notification_attention_active: Arc<AtomicBool>,
    pub keep_running_in_background: Arc<AtomicBool>,
}

impl AppState {
    pub fn new(
        store: Store,
        search: TantivySearch,
        crypto: CryptoService,
        snooze_stop_tx: std::sync::mpsc::Sender<()>,
        attachments_dir: PathBuf,
    ) -> Self {
        Self {
            store: Arc::new(store),
            search: Arc::new(search),
            crypto: Arc::new(crypto),
            oauth_account_locks: Arc::new(Mutex::new(HashMap::new())),
            secure_user_data_locks: Arc::new(Mutex::new(HashMap::new())),
            sync_handles: Mutex::new(HashMap::new()),
            snooze_stop_tx,
            attachments_dir,
            notifications_enabled: Arc::new(AtomicBool::new(true)),
            notification_attention_active: Arc::new(AtomicBool::new(false)),
            keep_running_in_background: Arc::new(AtomicBool::new(true)),
        }
    }
}
