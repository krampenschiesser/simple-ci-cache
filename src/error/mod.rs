use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Could not open path {0})")]
    OpenPathError(PathBuf),
}
