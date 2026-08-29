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

impl std::error::Error for BondarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        assert_eq!(
            BondarError::Config("bad".to_string()).to_string(),
            "Config error: bad"
        );
        assert_eq!(
            BondarError::NotFound("x".to_string()).to_string(),
            "Not found: x"
        );
        assert_eq!(
            BondarError::Docker("d".to_string()).to_string(),
            "Docker error: d"
        );
    }

    #[test]
    fn test_error_from_json() {
        let err: serde_json::Error = serde_json::from_str::<i32>("abc").unwrap_err();
        let bondar: BondarError = err.into();
        assert!(matches!(bondar, BondarError::Json(_)));
    }
}
