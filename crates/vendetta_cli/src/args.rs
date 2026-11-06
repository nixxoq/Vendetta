use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use vendetta_model::PeerType;
use vendetta_render::{DateStructure, MediaMode, PresentationMode, SplitBy, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    Warning = 1,
    Error = 2,
    Fatal = 3,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "vendetta",
    author = "NixxO",
    version = env!("CARGO_PKG_VERSION"),
    about = "Telegram chats archive & HTML exporter"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub account: Option<String>,

    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    #[command(name = "auth")]
    Auth {
        #[command(subcommand)]
        subcommand: Option<AuthSubcommands>,

        #[arg(long, global = true)]
        api_id: Option<i32>,

        #[arg(long, global = true)]
        api_hash: Option<String>,

        #[arg(long, global = true)]
        session: Option<PathBuf>,

        #[arg(long, global = true)]
        phone: Option<String>,

        #[arg(long, global = true)]
        force: bool,
    },

    #[command(name = "dialogs")]
    Dialogs {
        #[arg(long)]
        api_id: Option<i32>,

        #[arg(long)]
        api_hash: Option<String>,

        #[arg(long)]
        session: Option<PathBuf>,

        #[arg(long)]
        limit: Option<usize>,

        #[arg(long, value_enum)]
        peer_type: Option<CliPeerType>,
    },

    #[command(name = "sync")]
    Sync {
        #[arg(short, long)]
        archive: Option<PathBuf>,

        #[arg(long)]
        api_id: Option<i32>,

        #[arg(long)]
        api_hash: Option<String>,

        #[arg(long)]
        session: Option<PathBuf>,

        #[arg(long, value_delimiter = ',')]
        peers: Option<Vec<i64>>,

        #[arg(long, value_enum)]
        peer_type: Option<CliPeerType>,

        #[arg(long, value_enum)]
        exclude_peer_type: Option<CliPeerType>,

        #[arg(long)]
        limit: Option<usize>,

        #[arg(long = "from")]
        from_date: Option<String>,

        #[arg(long = "to")]
        to_date: Option<String>,
    },

    #[command(name = "download-media")]
    DownloadMedia {
        #[arg(short, long)]
        archive: Option<PathBuf>,

        #[arg(long)]
        media_dir: Option<PathBuf>,

        #[arg(long)]
        api_id: Option<i32>,

        #[arg(long)]
        api_hash: Option<String>,

        #[arg(long)]
        session: Option<PathBuf>,

        #[arg(long, default_value_t = false)]
        backfill: bool,

        #[arg(long, default_value_t = false)]
        avatars_only: bool,

        #[arg(long, default_value_t = false)]
        reactions_only: bool,

        #[arg(long, default_value_t = false)]
        photos_only: bool,

        #[arg(long, default_value_t = false)]
        skip_videos: bool,

        #[arg(long)]
        max_size_mb: Option<i64>,

        #[arg(long, default_value_t = 1)]
        min_workers: usize,

        #[arg(long, default_value_t = 8)]
        max_workers: usize,

        #[arg(long, default_value_t = 2)]
        max_dc_workers: usize,

        #[arg(long, default_value_t = 2)]
        initial_workers: usize,
    },

    #[command(name = "export-html")]
    ExportHtml {
        #[arg(short, long)]
        archive: PathBuf,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, value_enum, default_value_t = CliPresentationMode::TelegramLike)]
        mode: CliPresentationMode,

        #[arg(long, value_enum, default_value_t = CliMediaMode::Copy)]
        media: CliMediaMode,

        #[arg(long, value_enum, default_value_t = CliThemeMode::System)]
        theme: CliThemeMode,

        #[arg(long, default_value_t = 250)]
        chunk_size: usize,

        #[arg(long)]
        replace: bool,

        #[arg(long)]
        media_dir: Option<PathBuf>,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_service_messages: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_deleted_messages: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        include_edit_history: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_search_index: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_date_index: bool,

        #[arg(long, default_value_t = false)]
        disable_forum_render: bool,

        #[arg(long = "from")]
        from_date: Option<String>,

        #[arg(long = "to")]
        to_date: Option<String>,

        #[arg(long = "readable-names", default_value_t = false)]
        readable_names: bool,

        #[arg(long = "split-by", value_enum, default_value_t = CliSplitBy::Messages)]
        split_by: CliSplitBy,

        #[arg(long = "date-structure", value_enum, default_value_t = CliDateStructure::Flat)]
        date_structure: CliDateStructure,
    },

    #[command(name = "verify-html")]
    VerifyHtml {
        #[arg(long)]
        html_dir: PathBuf,
    },

    #[command(name = "verify-archive")]
    VerifyArchive {
        #[arg(short, long)]
        archive: Option<PathBuf>,

        #[arg(short = 'o', long = "html")]
        html_dir: Option<PathBuf>,

        #[arg(long, conflicts_with = "full")]
        fast: bool,

        #[arg(long, conflicts_with = "fast")]
        full: bool,

        #[arg(long)]
        media: bool,

        #[arg(long)]
        replies: bool,

        #[arg(long)]
        search: bool,

        #[arg(long)]
        media_dir: Option<PathBuf>,

        #[arg(long)]
        rehash: bool,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        json: bool,
    },

    #[command(name = "backfill-media")]
    BackfillMedia {
        #[arg(short, long)]
        archive: PathBuf,
    },

    #[command(name = "verify-media")]
    VerifyMedia {
        #[arg(short, long)]
        archive: PathBuf,

        #[arg(long)]
        media_dir: Option<PathBuf>,
    },

    #[command(name = "media-stats")]
    MediaStats {
        #[arg(short, long)]
        archive: PathBuf,
    },

    #[command(name = "requeue-skipped")]
    RequeueSkipped {
        #[arg(short, long)]
        archive: PathBuf,
    },

    #[command(name = "config")]
    Config {
        #[arg(long)]
        show: bool,

        #[arg(long)]
        api_id: Option<i32>,

        #[arg(long)]
        api_hash: Option<String>,

        #[arg(long)]
        account: Option<String>,

        #[arg(long)]
        archive: Option<PathBuf>,

        #[arg(long)]
        session: Option<PathBuf>,

        #[arg(long)]
        media_dir: Option<PathBuf>,

        #[arg(long)]
        output: Option<PathBuf>,

        #[arg(long)]
        base_dir: Option<PathBuf>,
    },

    #[command(name = "import")]
    Import {
        #[command(subcommand)]
        subcommand: ImportSubcommands,
    },

    #[command(name = "convert")]
    Convert {
        #[command(subcommand)]
        subcommand: ConvertSubcommands,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ImportSubcommands {
    #[command(name = "tdesktop")]
    Tdesktop {
        #[arg(short, long)]
        source: PathBuf,

        #[arg(short, long)]
        archive: PathBuf,

        #[arg(long)]
        media_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConvertSubcommands {
    #[command(name = "tdesktop")]
    Tdesktop {
        #[arg(short, long)]
        source: PathBuf,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(long, value_enum, default_value_t = CliPresentationMode::TelegramLike)]
        mode: CliPresentationMode,

        #[arg(long, value_enum, default_value_t = CliThemeMode::System)]
        theme: CliThemeMode,

        #[arg(long, default_value_t = 250)]
        chunk_size: usize,

        #[arg(long)]
        replace: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_search_index: bool,

        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        build_date_index: bool,

        #[arg(long, default_value_t = false)]
        disable_forum_render: bool,

        #[arg(long = "from")]
        from_date: Option<String>,

        #[arg(long = "to")]
        to_date: Option<String>,

        #[arg(long = "readable-names", default_value_t = false)]
        readable_names: bool,

        #[arg(long = "split-by", value_enum, default_value_t = CliSplitBy::Messages)]
        split_by: CliSplitBy,

        #[arg(long = "date-structure", value_enum, default_value_t = CliDateStructure::Flat)]
        date_structure: CliDateStructure,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSubcommands {
    #[command(name = "login")]
    Login,

    #[command(name = "status")]
    Status,

    #[command(name = "logout")]
    Logout {
        #[arg(long)]
        local_only: bool,
    },

    #[command(name = "forget")]
    Forget,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliPeerType {
    #[value(name = "user")]
    User,
    #[value(name = "group")]
    Group,
    #[value(name = "channel")]
    Channel,
}

impl From<CliPeerType> for PeerType {
    fn from(p: CliPeerType) -> Self {
        match p {
            CliPeerType::User => PeerType::User,
            CliPeerType::Group => PeerType::Group,
            CliPeerType::Channel => PeerType::Channel,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliPresentationMode {
    #[value(name = "telegram-like")]
    TelegramLike,
    #[value(name = "archive-optimized")]
    ArchiveOptimized,
}

impl From<CliPresentationMode> for PresentationMode {
    fn from(m: CliPresentationMode) -> Self {
        match m {
            CliPresentationMode::TelegramLike => PresentationMode::TelegramLike,
            CliPresentationMode::ArchiveOptimized => PresentationMode::ArchiveOptimized,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliMediaMode {
    #[value(name = "copy")]
    Copy,
    #[value(name = "link")]
    Link,
}

impl From<CliMediaMode> for MediaMode {
    fn from(m: CliMediaMode) -> Self {
        match m {
            CliMediaMode::Copy => MediaMode::Copy,
            CliMediaMode::Link => MediaMode::Link,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliThemeMode {
    #[value(name = "light")]
    Light,
    #[value(name = "dark")]
    Dark,
    #[value(name = "system")]
    System,
}

impl From<CliThemeMode> for ThemeMode {
    fn from(m: CliThemeMode) -> Self {
        match m {
            CliThemeMode::System => ThemeMode::System,
            CliThemeMode::Light => ThemeMode::Light,
            CliThemeMode::Dark => ThemeMode::Dark,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CliSplitBy {
    #[value(name = "messages")]
    #[default]
    Messages,
    #[value(name = "day")]
    Day,
}

impl From<CliSplitBy> for SplitBy {
    fn from(s: CliSplitBy) -> Self {
        match s {
            CliSplitBy::Messages => SplitBy::Messages,
            CliSplitBy::Day => SplitBy::Day,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CliDateStructure {
    #[value(name = "flat")]
    #[default]
    Flat,
    #[value(name = "tree")]
    Tree,
}

impl From<CliDateStructure> for DateStructure {
    fn from(s: CliDateStructure) -> Self {
        match s {
            CliDateStructure::Flat => DateStructure::Flat,
            CliDateStructure::Tree => DateStructure::Tree,
        }
    }
}