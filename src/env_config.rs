use std::{collections::BTreeMap, env};

use smol_str::{SmolStr, ToSmolStr};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct EnvConfig {
    pub config_file_name: SmolStr,
    pub cache_dir: Option<SmolStr>,
    pub read_only: bool,
}
pub fn parse_env() -> EnvConfig {
    let env_vars = env::vars().collect::<BTreeMap<String, String>>();

    debug!("From environment vars:");
    let config_file_name = env_vars
        .get("CACHE_CONFIG_FILE")
        .map(|e| e.to_smolstr())
        .unwrap_or("cache.yml".to_smolstr());
    debug!("config_filename=\"{}\"", config_file_name);

    let cache_dir = env_vars.get("CACHE_DIR").map(|e| e.to_smolstr());
    debug!("cache_dir=\"{:?}\"", cache_dir);

    let read_only = env_vars
        .get("CACHE_RO")
        .map(|e| e == "true")
        .unwrap_or(false);
    debug!("cache_readonly=\"{}\"", read_only);

    return EnvConfig {
        read_only,
        cache_dir,
        config_file_name,
    };
}
