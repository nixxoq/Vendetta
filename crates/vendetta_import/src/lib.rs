pub mod converter;
pub mod discovery;
pub mod error;
pub mod ingest;
pub mod model;
pub mod parser_html;
pub mod parser_json;
pub mod self_identity;
pub mod synthetic_id;

pub use converter::{
    ConvertOptions, ConvertSummary, ImportOptions, ImportSummary, convert_tdesktop, import_tdesktop,
};
pub use error::{ImportError, ImportResult};
pub use model::{
    ImportChat, ImportEntityKind, ImportForwardInfo, ImportMediaItem, ImportMessage,
    ImportReaction, ImportServiceEvent, ImportTextEntity,
};
pub use synthetic_id::{
    SYNTHETIC_CHAT_ID_BASE, SYNTHETIC_MODULO, SYNTHETIC_SENDER_ID_BASE, generate_synthetic_chat_id,
    generate_synthetic_sender_id,
};
