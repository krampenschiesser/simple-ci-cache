use anyhow::Context;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use std::{
    collections::BTreeMap,
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
    vec::Vec,
};
use tracing::{debug, info, trace};

use crate::env_config::EnvConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectId(pub SmolStr);

impl Deref for ProjectId {
    type Target = SmolStr;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for ProjectId {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub root: SmolStr,
    #[serde(default)]
    pub envs: Vec<SmolStr>,
    #[serde(default)]
    pub inputs: Vec<SmolStr>,
    #[serde(default)]
    pub outputs: Vec<SmolStr>,
    pub name: ProjectId,
    #[serde(default)]
    pub depends_on: Vec<ProjectId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionEnvironment {
    BASH,
    SHELL,
}

impl AsRef<str> for ExecutionEnvironment {
    fn as_ref(&self) -> &str {
        match &self {
            ExecutionEnvironment::BASH => "bash",
            ExecutionEnvironment::SHELL => "shell",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub exec: ExecutionEnvironment,
    pub projects: Vec<Project>,
    pub cache_dir: SmolStr,
    pub ttl: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exec: ExecutionEnvironment::BASH,
            projects: Default::default(),
            cache_dir: ".cache".into(),
            ttl: 7,
        }
    }
}

impl Config {
    pub fn discover_file(env_config: &EnvConfig) -> anyhow::Result<Option<PathBuf>> {
        let mut cwd = env::current_dir()?;
        let mut should_continue = true;
        while should_continue {
            let config_file_path = cwd.join(env_config.config_file_name.as_str());
            debug!("checking for config in {:?}", config_file_path);
            if fs::exists(&config_file_path)? {
                info!("Using configuration file {:?}", config_file_path);
                return Ok(Some(config_file_path.canonicalize()?));
            }

            should_continue = cwd.pop();
        }
        info!("Could not find configuration");
        Ok(None)
    }
    pub fn filter_env_vars(
        &self,
        env: &BTreeMap<String, String>,
        root_path: &Path,
    ) -> anyhow::Result<BTreeMap<String, String>> {
        let project = self.get_project_for_cwd(root_path)?;
        let mut result = BTreeMap::new();

        if let Some(project) = project {
            for env_var_name in &project.envs {
                let value = env.get(env_var_name.as_str());
                if let Some(value) = value {
                    result.insert(env_var_name.to_string(), value.into());
                }
            }
        }
        Ok(result)
    }

    pub fn get_project(&self, id: &ProjectId) -> Option<&Project> {
        self.projects.iter().find(|p| &p.name == id)
    }

    pub fn get_project_for_cwd(&self, root: &Path) -> anyhow::Result<Option<&Project>> {
        let cwd = env::current_dir()?
            .canonicalize()
            .with_context(|| format!("Could not canonicalize cwd"))?;
        for project in &self.projects {
            let project_path = root.join(project.root.as_str());
            let project_path = project_path
                .canonicalize()
                .with_context(|| format!("Could not canonicalize {:?}", project_path))?;
            if cwd.starts_with(&project_path) {
                trace!(
                    "Checking if project path {:?} is in {:?}",
                    &project_path, cwd
                );
                return Ok(Some(project));
            }
        }
        return Ok(None);
    }
}
