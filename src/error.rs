use std::fmt;

#[derive(Debug)]
pub enum BondarError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Config(String),
    Docker(String),
    NotFound(String),
}

impl fmt::Display for BondarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {e}"),
            Self::Json(e) => write!(f, "JSON error: {e}"),
            Self::Config(msg) => write!(f, "Config error: {msg}"),
            Self::Docker(msg) => write!(f, "Docker error: {msg}"),
            Self::NotFound(msg) => write!(f, "Not found: {msg}"),
        }
    }
}

impl std::error::Error for BondarError {}

impl From<std::io::Error> for BondarError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for BondarError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

pub type Result<T> = std::result::Result<T, BondarError>;
