pub mod apps;
pub mod auth;
pub mod cli;
pub mod client;
pub mod cost;
pub mod directory;
pub mod management_groups;
pub mod models;
pub mod resource_graph;
#[cfg(any(test, feature = "test-util"))]
pub mod testing;
pub mod timing;

pub use auth::{AuthProvider, AzureAuth};
pub use client::AzureClient;
pub use resource_graph::{GraphScope, ResourceGraphClient};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AzureError {
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    #[error("API request failed: {0}")]
    ApiError(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("HTTP {status}: {message}")]
    Http { status: u16, message: String },

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

pub type Result<T> = std::result::Result<T, AzureError>;
