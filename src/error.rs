#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Config(Box<figment::Error>),
}

impl From<figment::Error> for Error {
    fn from(value: figment::Error) -> Self {
        Self::Config(Box::new(value))
    }
}

impl Error {
    pub fn message(value: impl Into<String>) -> Self {
        Self::Message(value.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
