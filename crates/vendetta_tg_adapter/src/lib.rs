pub mod adapter;
pub mod auth;
pub mod error;
pub mod fake;
pub mod normalize;
pub mod session;
pub mod traits;

pub use adapter::GrammersTelegramAdapter;
pub use auth::{AuthPrompt, TelegramAuthService};
pub use error::{AdapterError, AdapterResult};
pub use fake::FakeTelegramAdapter;
pub use normalize::{
    extract_media_records, normalize_channel, normalize_dialog, normalize_group, normalize_message,
    normalize_peer, normalize_peer_enum, normalize_raw_chat, normalize_raw_user, normalize_update,
    normalize_user,
};
pub use session::{
    FileSession, FileSessionError, SessionState, load_session_file, save_session_file,
};
pub use traits::{
    ChannelDifferenceResult, CommonDifferenceResult, DialogDiscoveryResult, DialogsPage,
    HistoryPage, TelegramAdapter,
};
