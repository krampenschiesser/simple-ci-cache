use std::{fs, path::Path};

use crate::config::Config;
use anyhow::Context;
use smol_str::SmolStr;

pub fn parse_config_file(
    path: &Path,
    cache_dir_override: Option<SmolStr>,
) -> anyhow::Result<Config> {
    if path.exists() {
        let yaml =
            fs::read_to_string(path).with_context(|| format!("Could not read {:?}", path))?;
        let mut config: Config = serde_yml::from_str(&yaml)
            .with_context(|| format!("Could not parse config yaml for {:?}", path))?;
        if let Some(cache_dir) = cache_dir_override {
            config.cache_dir = cache_dir;
        }
        Ok(config)
    } else {
        Ok(Config::default())
    }
}
