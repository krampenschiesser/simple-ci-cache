use std::{
    collections::BTreeMap,
    env::{self},
    fs,
    path::PathBuf,
    process::Stdio,
};

use anyhow::{Context, bail};
use blake3::Hash;
use chrono::Utc;
use clap::Parser;
use simple_ci_cache::{
    cache::{
        command::CachedCommand, file::CachedFile, folder::CacheFolder, glob::get_paths_from_globs,
    },
    cli::CommandLineArgs,
    config::{parse::parse_config_file, types::Config},
    env_config::parse_env,
    standard_out::redirect_to_file_and_stdout,
};
use smol_str::ToSmolStr;
use tokio::task::JoinSet;
use tracing::{debug, info};
use tracing_subscriber::{filter, fmt, layer::SubscriberExt, reload, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = filter::LevelFilter::INFO;
    let (filter, reload_handle) = reload::Layer::new(filter);
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::Layer::default())
        .init();

    let cli = CommandLineArgs::parse();
    if cli.verbose {
        reload_handle
            .modify(|filter| *filter = filter::LevelFilter::TRACE)
            .expect("Could not change log level to DEBUG");
    }

    let env_vars = env::vars().collect::<BTreeMap<String, String>>();

    let env_config = parse_env();
    let config_path =
        Config::discover_file(&env_config).with_context(|| "Failed to discover config file")?;
    let maybe_config_path = cli.config.map(|c| PathBuf::from(c)).or(config_path);
    let (config, root_path) = if let Some(config_path) = maybe_config_path {
        let config = parse_config_file(&config_path, env_config.cache_dir)
            .with_context(|| format!("failed to parse config file from {:?}", config_path))?;
        debug!("Using configuration {:?}", &config);
        (
            config,
            config_path
                .parent()
                .map(|p| p.to_owned())
                .expect("Could not get root folder of cache"),
        )
    } else {
        let config = Config::default();
        let dir = env::current_dir()?;
        debug!("Using default configuration {:?}", &config);
        (config, dir)
    };

    let cache_folder_path = root_path.join(config.cache_dir.as_str());
    fs::create_dir_all(&cache_folder_path)?;
    let cache_folder_path = cache_folder_path.canonicalize()?;
    debug!("Using cache path {:?}", &cache_folder_path);
    if cli.clear {
        info!("Clearing cache {:?}", &cache_folder_path);
        fs::remove_dir_all(&cache_folder_path)?;
    }
    let project = config.get_project_for_cwd(&root_path)?;
    let mut inputs = Vec::new();
    let mut projects = Vec::new();
    if let Some(project) = project {
        info!("Operating in project {}", project.name.0);
        projects.push(project);
        while let Some(current_project) = projects.pop() {
            // add depenent projects
            current_project
                .depends_on
                .iter()
                .filter_map(|id| config.get_project(id))
                .for_each(|p| {
                    debug!("Found dependency {}", project.name.as_ref());
                    projects.push(p);
                });
            debug!(
                "Adding {} inputs from project {}",
                inputs.len(),
                current_project.name.as_ref()
            );
            inputs.extend(current_project.inputs.clone());
        }
    }

    let all_paths = get_paths_from_globs(&inputs, &root_path);

    let filtered_env = config.filter_env_vars(&env_vars, &root_path)?;
    debug!("Filtered env: {:?}", &filtered_env);

    let command_string = cli.command.join(" ");

    let command_hash = CachedCommand::create_hash(&command_string, all_paths)?;
    debug!(
        "Computed command hash {} for '{}'",
        command_hash.to_string(),
        command_string
    );
    let cache_folder = CacheFolder::new(cache_folder_path);
    if cache_folder.has_cached_command(&command_hash) {
        let command = cache_folder.get_cashed_command(&command_hash)?;
        info!("Cache hit for {}", &command_string);

        let cached_output = cache_folder
            .get_cached_file(&Hash::from_hex(command.log.as_bytes())?)
            .await?;
        let mut set = JoinSet::new();

        let stdout_future = cached_output.restore_to_stdout();
        set.spawn(stdout_future);

        for file_hash_str in command.output_files {
            let file_hash = Hash::from_hex(file_hash_str.as_bytes())?;
            let file = cache_folder.get_cached_file(&file_hash).await?;
            let restore_future = file.restore();
            set.spawn(restore_future);
        }

        while let Some(res) = set.join_next().await {
            match res {
                Err(e) => bail!(e),
                Ok(_) => {}
            }
        }
    } else {
        let temp_file_path = std::env::temp_dir().join(format!("{}.txt", command_hash.to_string()));

        let shell_command = config.exec.as_ref();
        let mut process = tokio::process::Command::new(shell_command);
        process.arg("-c");
        process.arg(&command_string);
        process.stdout(Stdio::piped());

        let mut child = process.spawn()?;
        let child_output = child.stdout.take();
        if let Some(child_stdout) = child_output {
            tokio::spawn(redirect_to_file_and_stdout(
                temp_file_path.clone(),
                child_stdout,
            ));
        } else {
            bail!("Could not capture command output")
        }
        child.wait().await?;

        let mut output_hashes = Vec::new();
        let command_line_output_hash =
            CachedFile::create(cache_folder.root.clone(), temp_file_path).await?;
        if let Some(project) = project {
            let paths = get_paths_from_globs(&project.outputs, &root_path);
            let mut futures = JoinSet::new();
            for path in paths.clone() {
                let future = CachedFile::create(cache_folder.root.clone(), path);
                futures.spawn(future);
            }
            while let Some(res) = futures.join_next().await {
                match res {
                    Err(e) => bail!(e),
                    Ok(hash) => match hash {
                        Err(e) => bail!(e),
                        Ok(hash) => output_hashes.push(hash.to_smolstr()),
                    },
                }
            }
        }
        let cached_command = CachedCommand {
            command_line: command_string.into(),
            created: Utc::now(),
            env: filtered_env.into(),
            hash: command_hash.to_string().into(),
            last_accessed: Utc::now(),
            log: command_line_output_hash.to_string().into(),
            output_files: output_hashes,
        };
        cached_command.store_in_cache(&cache_folder.root)?;
    }
    Ok(())
}
