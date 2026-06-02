use thiserror::Error;
use tokio::io;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("listen on addr err: {0}")]
    ListenError(String),

    #[error("server io error: {0}")]
    IOError(#[from] io::Error),

    #[error("parse http err: {0}")]
    HTTPParseError(#[from] httparse::Error),
}
