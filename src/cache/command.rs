use anyhow::Ok;
use blake3::Hash;
use chrono::{DateTime, Utc};

use rayon::slice::ParallelSliceMut;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::{fs::File, io::AsyncWriteExt};
use tracing::debug;

pub const COMMAND_DIR: &'static str = "commands";
pub const COMMAND_FILE_NAME: &'static str = "command.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputFile {
    pub paths: Vec<SmolStr>,
    pub hash: SmolStr,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedCommand {
    pub command_line: SmolStr,
    pub env: BTreeMap<String, String>,
    pub hash: SmolStr,
    pub created: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub log: SmolStr,
    pub output_files: Vec<OutputFile>,
}

impl CachedCommand {
    pub fn create_hash(
        commandline: &str,
        mut files: Vec<PathBuf>,
        filtered_env: &BTreeMap<String, String>,
    ) -> anyhow::Result<Hash> {
        files.par_sort_by_key(|e| e.canonicalize().expect("full path")); //fixme

        let mut hasher = blake3::Hasher::new();
        hasher.update(commandline.as_ref());

        for (k, v) in filtered_env {
            hasher.update(k.as_bytes());
            hasher.update(v.as_bytes());
        }

        let amount = files.len();
        let start = SystemTime::now();
        for file in files {
            hasher.update_mmap_rayon(&file)?;
        }
        let result = hasher.finalize();

        let elapsed = start.elapsed()?;
        if amount > 0 {
            debug!("Hashing {} files took {:?}", amount, elapsed);
        }

        Ok(result)
    }

    pub async fn store_in_cache(self, cache_dir: &Path) -> anyhow::Result<()> {
        let json = serde_json::to_string(&self)?;
        let target_folder = cache_dir.join(COMMAND_DIR).join(self.hash.to_string());
        tokio::fs::create_dir_all(&target_folder).await?;
        let mut file = File::create(target_folder.join(COMMAND_FILE_NAME)).await?;
        file.write_all(json.as_bytes()).await?;
        Ok(())
    }
}
