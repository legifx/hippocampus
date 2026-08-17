pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error(
        "the index was written by a newer version of Hippocampus (schema {found}, this build \
         supports {supported}). Upgrade, or delete the index and let it rebuild."
    )]
    SchemaFromFuture { found: i64, supported: i64 },

    #[error("a stored value could not be interpreted: {0}")]
    Corrupt(String),

    #[error(transparent)]
    Core(#[from] cortex_core::Error),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("could not serialise value: {0}")]
    Json(#[from] serde_json::Error),
}

impl StoreError {
    pub fn corrupt(msg: impl Into<String>) -> Self {
        StoreError::Corrupt(msg.into())
    }
}
