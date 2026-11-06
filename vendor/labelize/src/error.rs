use thiserror::Error;

#[derive(Error, Debug)]
pub enum LabelizeError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("Encode error: {0}")]
    Encode(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, LabelizeError>;
