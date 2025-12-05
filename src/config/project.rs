use anyhow::{Context, bail};
use blake3::Hash;
use itertools::Itertools;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, ToSmolStr};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{sync::Semaphore, task::JoinSet};
use tracing::debug;

use crate::cache::{
    command::OutputFile, file::CachedFile, folder::CacheFolder, glob::get_paths_from_globs,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub root: SmolStr,
    #[serde(default)]
    pub envs: Vec<SmolStr>,
    #[serde(default)]
    pub inputs: Vec<SmolStr>,
    #[serde(default)]
    pub outputs: Vec<SmolStr>,
    pub name: SmolStr,
    #[serde(default)]
    pub depends_on: Vec<SmolStr>,
}
impl Project {
    pub async fn gather_output_files(
        &self,
        root_folder: &Path,
        cache_folder: &CacheFolder,
    ) -> anyhow::Result<Vec<OutputFile>> {
        let paths = get_paths_from_globs(&self.outputs, &root_folder)
            .into_iter()
            .unique()
            .collect::<Vec<PathBuf>>();

        let mut output_path_map: HashMap<Hash, (NonEmpty<PathBuf>, u64)> = HashMap::new();

        for path in paths {
            let (hash, size) = CachedFile::hash_path(&path)
                .with_context(|| format!("Could not hash file {:?}", &path))?;

            output_path_map
                .entry(hash)
                .and_modify(|e| e.0.push(path.clone()))
                .or_insert((NonEmpty::new(path.clone()), size));
        }
        let mut futures = JoinSet::<anyhow::Result<(NonEmpty<PathBuf>, SmolStr)>>::new();
        let semaphore = Arc::new(Semaphore::new(100));

        for (hash, (paths, size)) in output_path_map {
            let hash_string = hash.to_smolstr();
            let future =
                CachedFile::create(cache_folder.root.clone(), paths.first().clone(), hash, size);
            let clone = semaphore.clone();
            futures.spawn(async move {
                let _token = clone.acquire().await?;
                future.await?;
                Ok((paths, hash_string))
            });
        }

        let mut output_files = Vec::new();
        while let Some(res) = futures.join_next().await {
            match res {
                Err(e) => bail!(e),
                Ok(hash) => match hash {
                    Err(e) => bail!(e),
                    Ok((paths, hash)) => {
                        debug!("Successfully hashed {} to {:?}", &hash, &paths);
                        output_files.push(OutputFile {
                            hash,
                            paths: paths
                                .into_iter()
                                .map(|p| p.to_string_lossy().to_smolstr())
                                .collect(),
                        });
                    }
                },
            }
        }
        Ok(output_files)
    }
}
