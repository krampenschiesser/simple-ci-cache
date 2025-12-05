use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Ok, bail};
use async_compression::tokio::bufread::{BrotliDecoder, BrotliEncoder, XzDecoder, XzEncoder};
use blake3::Hash;
use chrono::{DateTime, Utc};
use file_type::FileType;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, ToSmolStr};
use tokio::{
    fs::{File, create_dir_all},
    io::{AsyncWriteExt, BufReader, BufWriter, copy, copy_buf, stdout},
};
use tracing::{debug, trace};

use crate::{cache::folder::FILE_FOLDER_NAME, error::CacheError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Compression {
    None,
    Brotli,
    Xz,
    XzParallel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCacheFile {
    pub created: DateTime<Utc>,
    pub original_hash: SmolStr,
    pub compression: Compression,
}

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub path: PathBuf,
    pub data: StoredCacheFile,
}
pub const COMPRESSED_FILE_NAME: &'static str = "compressed";
pub const DATA_FILE_NAME: &'static str = "file.json";

impl CachedFile {
    pub fn hash_path(path: &Path) -> anyhow::Result<(Hash, u64)> {
        let mut hasher = blake3::Hasher::new();
        let start = SystemTime::now();
        // blake 3 does file size check already and uses best way to hash (readfile,mmap,parallel)
        hasher.update_mmap_rayon(&path)?;

        let hash = hasher.finalize();
        let elapsed = start.elapsed().expect("Could not measure system time");
        let bytes_hashed = hasher.count();
        debug!(
            "Hashing {} bytes {:?} took {:?}",
            bytes_hashed, path, elapsed
        );
        Ok((hash, bytes_hashed))
    }

    fn to_file_cache_dir(path: &Path) -> PathBuf {
        if path.ends_with(FILE_FOLDER_NAME) {
            path.to_owned()
        } else {
            path.join(FILE_FOLDER_NAME)
        }
    }

    fn determine_compression(path: &Path, file_size: u64) -> anyhow::Result<Compression> {
        if file_size < 10 * 1024 {
            trace!(
                "File is too small to even deal with compression {:?} -> as-is",
                path
            );
            return Ok(Compression::None);
        }

        let file_type = FileType::try_from_file(path)?;
        let media_types = file_type.media_types().join(", ");
        if media_types.contains(&"text") {
            trace!("Discovered text file {:?} -> compress brotli", path);
            Ok(Compression::Brotli)
        } else if ["zip", "image", "video", "audio", "archive"]
            .iter()
            .any(|contains| media_types.contains(contains))
        {
            trace!(
                "Media type {} marks this as already compressed file {:?} -> as-is",
                media_types, path
            );
            Ok(Compression::None)
        } else if file_size > 1024 * 1024 * 1024 {
            trace!(
                ">1gb and Media type {} suggest this file needs compression {:?} -> compress xz parallel",
                media_types, path
            );
            Ok(Compression::XzParallel)
        } else {
            trace!(
                "Media type {} suggest this file needs compression {:?} -> compress xz",
                media_types, path
            );
            Ok(Compression::Xz)
        }
    }

    pub async fn create(
        cache_dir: PathBuf,
        original_path: PathBuf,
        hash: Hash,
        size: u64,
    ) -> anyhow::Result<Hash> {
        let cache_dir = Self::to_file_cache_dir(&cache_dir);
        let file_dir = cache_dir.join(hash.to_string());
        if file_dir.exists() {
            debug!("File with hash {} already cached", hash);
            return Ok(hash);
        } else {
            create_dir_all(&file_dir).await?;
        }
        let compression = Self::determine_compression(&original_path, size)?;

        let original = File::open(&original_path).await?;
        let mut target = File::create_new(file_dir.join(COMPRESSED_FILE_NAME)).await?;
        let mut reader = BufReader::new(original);
        match compression {
            Compression::Brotli => {
                let mut encoder = BrotliEncoder::new(reader);
                copy(&mut encoder, &mut target).await?;
            }
            Compression::None => {
                copy_buf(&mut reader, &mut target).await?;
            }
            Compression::Xz => {
                let mut encoder = XzEncoder::new(reader);
                copy(&mut encoder, &mut target).await?;
            }
            Compression::XzParallel => {
                let threads =
                    NonZeroU32::new(num_cpus::get_physical() as u32 - 1).expect("0 cores? errr...");
                let mut encoder =
                    XzEncoder::parallel(reader, async_compression::Level::Best, threads);
                copy(&mut encoder, &mut target).await?;
            }
        }
        let data = StoredCacheFile {
            compression,
            created: Utc::now(),
            original_hash: hash.to_smolstr(),
        };
        let mut data_file = File::create_new(file_dir.join(DATA_FILE_NAME)).await?;

        let json = serde_json::to_string(&data)?;
        data_file.write_all(&json.as_bytes()).await?;

        target.flush().await?;
        data_file.flush().await?;
        Ok(hash)
    }

    pub fn open(cache_dir: &Path, hash: &Hash) -> anyhow::Result<Self> {
        let cache_dir = Self::to_file_cache_dir(cache_dir);
        let hex = hash.to_string();
        let target_folder = cache_dir.join(hex);
        let json_file = target_folder.join(DATA_FILE_NAME);
        let binary_file = target_folder.join(COMPRESSED_FILE_NAME);
        for path in [&target_folder, &json_file, &binary_file] {
            if !path.exists() {
                bail!(CacheError::OpenPathError(path.to_owned()))
            }
        }
        let json_file = std::fs::File::open(&json_file)?;
        let data: StoredCacheFile = serde_json::from_reader(&json_file)?;

        Ok({
            Self {
                path: binary_file,
                data,
            }
        })
    }

    pub async fn create_parent(path: &Path) {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                debug!(
                    "Creating parent folder structure to restore file {:?}",
                    parent
                );
                let res = create_dir_all(parent).await;
                if let Err(e) = res {
                    trace!(
                        "Multiple threads tried to create parent folder structure, ignore. {}",
                        e
                    )
                }
            }
        }
    }

    pub async fn restore(
        self,
        destinations: NonEmpty<PathBuf>,
    ) -> anyhow::Result<NonEmpty<PathBuf>> {
        let read_file = File::open(&self.path)
            .await
            .with_context(|| format!("failed to open cached file binary {:?}", &self.path))?;

        for destination in &destinations {
            Self::create_parent(destination).await;
        }

        let original_path = destinations.first();
        let mut buf_read = BufReader::new(read_file);

        let mut write_file = File::create(&original_path).await.with_context(|| {
            format!(
                "creating output file for cached file failed: {:?}",
                original_path
            )
        })?;

        match &self.data.compression {
            Compression::Brotli => {
                let mut decoder = BrotliDecoder::new(buf_read);
                copy(&mut decoder, &mut write_file).await?;
            }
            Compression::None => {
                copy_buf(&mut buf_read, &mut write_file).await?;
            }
            Compression::XzParallel => {
                let mut decoder = XzDecoder::parallel_with_mem_limit(
                    buf_read,
                    NonZeroU32::new(num_cpus::get_physical() as u32 - 1).expect("0 cores? errr..."),
                    256 * 1024 * 1024,
                );
                copy(&mut decoder, &mut write_file).await?;
            }

            Compression::Xz => {
                let mut decoder = XzDecoder::with_mem_limit(buf_read, 256 * 1024 * 1024);
                copy(&mut decoder, &mut write_file).await?;
            }
        };
        for dest in destinations.tail() {
            let source_file = File::open(original_path).await?;
            let dest_file = File::create(dest).await?;
            let mut writer = BufWriter::new(dest_file);
            let mut reader = BufReader::new(source_file);
            copy_buf(&mut reader, &mut writer).await?;
        }

        Ok(destinations)
    }

    pub async fn restore_to_stdout(self) -> anyhow::Result<()> {
        let read_file = File::open(&self.path).await?;
        let mut buf_read = BufReader::new(read_file);

        match &self.data.compression {
            Compression::Brotli => {
                let mut decoder = BrotliDecoder::new(buf_read);
                copy(&mut decoder, &mut stdout()).await?;
            }
            Compression::None => {
                copy_buf(&mut buf_read, &mut stdout()).await?;
            }
            Compression::XzParallel => {
                let mut decoder = XzDecoder::parallel_with_mem_limit(
                    buf_read,
                    NonZeroU32::new(num_cpus::get_physical() as u32 - 1).expect("0 cores? errr..."),
                    256 * 1024 * 1024,
                );
                copy(&mut decoder, &mut stdout()).await?;
            }
            Compression::Xz => {
                let mut decoder = XzDecoder::with_mem_limit(buf_read, 256 * 1024 * 1024);
                copy(&mut decoder, &mut stdout()).await?;
            }
        };

        Ok(())
    }
}
