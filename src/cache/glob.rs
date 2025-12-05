use std::path::{Path, PathBuf};

use glob::glob;
use itertools::Itertools;
use smol_str::SmolStr;
use tracing::{error, trace};

pub fn get_paths_from_globs(glob_strings: &[SmolStr], root_dir: &Path) -> Vec<PathBuf> {
    let paths = glob_strings
        .iter()
        .filter_map(|pattern| {
            let full_pattern = format!(
                "{}/{}",
                root_dir
                    .canonicalize()
                    .expect("Couldnt canonicalize glob path")
                    .to_string_lossy(),
                pattern
            );
            trace!("Checking glob {}", full_pattern);
            let result = glob(&full_pattern);
            match result {
                Err(e) => {
                    error!("Could not parse glob pattern {}: {}", pattern, e);
                    None
                }
                Ok(paths) => Some(paths),
            }
        })
        .flat_map(|v| v.into_iter())
        .filter_map(|p| match p {
            Err(e) => {
                error!("Invalid glob: {}", e);
                None
            }
            Ok(path) => {
                trace!("Found path matching glob: {:?}", path);
                Some(path)
            }
        })
        .filter(|p| p.is_file())
        .unique()
        .collect();
    paths
}
