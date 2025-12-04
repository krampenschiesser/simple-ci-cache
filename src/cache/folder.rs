use std::{
    fs::{self, File},
    path::PathBuf,
};

use anyhow::bail;
use tracing::error;

use crate::cache::{
    command::{COMMAND_DIR, COMMAND_FILE_NAME, CachedCommand},
    file::CachedFile,
};

pub const FILE_FOLDER_NAME: &'static str = "files";
pub struct CacheFolder {
    pub root: PathBuf,
}

impl CacheFolder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn has_cached_file(&self, hash: &blake3::Hash) -> bool {
        fs::exists(self.root.join(FILE_FOLDER_NAME).join(&hash.to_string())).is_ok()
    }

    pub async fn get_cached_file(&self, hash: &blake3::Hash) -> anyhow::Result<CachedFile> {
        CachedFile::open(&self.root, hash)
    }

    pub fn has_cached_command(&self, hash: &blake3::Hash) -> bool {
        let exists = fs::exists(self.root.join(COMMAND_DIR).join(&hash.to_string()));
        exists.expect("Could not find cache file")
    }

    pub fn get_cashed_command(
        &self,
        hash: &blake3::Hash,
    ) -> anyhow::Result<crate::cache::command::CachedCommand> {
        let command_folder = self.root.join(COMMAND_DIR).join(hash.to_string());
        if !command_folder.exists() {
            bail!("Could not find cached command {}", hash);
        }

        let json_file = command_folder.join(COMMAND_FILE_NAME);
        if !json_file.exists() {
            bail!(
                "Found command folder {:?} but no \"{}\"",
                command_folder,
                COMMAND_FILE_NAME
            );
        }

        let command: CachedCommand = serde_json::from_reader(File::open(json_file)?)?;
        Ok(command)
    }
}
