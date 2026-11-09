pub mod app;
pub mod app_view;
pub mod apps;
pub mod aws;
pub mod cache;
pub mod complete;
pub mod config;
pub mod detail;
pub mod export;
pub mod fuzzy;
pub mod gcp;
pub mod loader;
pub mod models;
pub mod palette;
pub mod paths;
pub mod policy;
pub mod provider;
pub mod query;
pub mod rbac;
pub mod resource_type;
pub mod safe_text;
pub mod scope_tree;
pub mod sort;
pub mod state;
/// Timing stats (defined in c9s-azure so its HTTP clients can record too).
pub use c9s_azure::timing;

pub use app::App;
pub use state::AppState;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CoreError {
    #[error("Cache error: {0}")]
    Cache(#[from] rusqlite::Error),

    #[error("Azure error: {0}")]
    Azure(#[from] c9s_azure::AzureError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Invalid state: {0}")]
    InvalidState(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
