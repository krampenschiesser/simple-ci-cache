use std::{
    collections::BTreeMap,
    env::{self},
    fs,
    path::PathBuf,
    process::Stdio,
};

use anyhow::{Context, anyhow, bail};
use blake3::Hash;
use chrono::Utc;
use clap::Parser;
use itertools::Itertools;
use nonempty::NonEmpty;
use simple_ci_cache::{
    cache::{
        command::CachedCommand, file::CachedFile, folder::CacheFolder, glob::get_paths_from_globs,
    },
    cli::CommandLineArgs,
    config::{Config, parse::parse_config_file, project::Project},
    env_config::parse_env,
    standard_out::redirect_to_file_and_stdout,
};
use tokio::task::JoinSet;
use tracing::{debug, info};
use tracing_subscriber::{
    filter,
    fmt::{self},
    layer::SubscriberExt,
    reload,
    util::SubscriberInitExt,
};

async fn initialize(cli: &CommandLineArgs) -> anyhow::Result<(Config, PathBuf, PathBuf)> {
    let env_config = parse_env();
    let config_path =
        Config::discover_file(&env_config).with_context(|| "Failed to discover config file")?;
    let maybe_config_path = cli
        .config
        .as_ref()
        .map(|c| PathBuf::from(c))
        .or(config_path);
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
    debug!("Using cache folder {:?}", &cache_folder_path);

    Ok((config, root_path, cache_folder_path))
}
async fn handle_existing_command(
    command_hash: Hash,
    command_string: &str,
    cache_folder: CacheFolder,
) -> anyhow::Result<()> {
    let command = cache_folder.get_cashed_command(&command_hash)?;
    info!(
        "Cache hit for {} [{}]",
        command_hash.to_hex(),
        &command_string
    );

    let cached_output = cache_folder
        .get_cached_file(&Hash::from_hex(command.log.as_bytes())?)
        .await?;
    let stdout_future = cached_output.restore_to_stdout();

    let mut set = JoinSet::new();
    for output_file in command.output_files {
        let file_hash = Hash::from_hex(output_file.hash.as_bytes())?;
        let file = cache_folder.get_cached_file(&file_hash).await?;
        let paths = output_file
            .paths
            .into_iter()
            .map(|s| PathBuf::from(s.as_str()))
            .collect();
        let dests =
            NonEmpty::from_vec(paths).ok_or(anyhow!("need at least one output paths, got 0"))?;

        let restore_future = file.restore(dests);
        set.spawn(restore_future);
    }
    stdout_future.await?;

    while let Some(res) = set.join_next().await {
        match res {
            Err(e) => bail!(e),
            Ok(task_result) => match task_result {
                Err(e) => bail!(e),
                Ok(file_name) => debug!("Restored files {:?}", file_name),
            },
        }
    }
    Ok(())
}
async fn handle_new_command(
    command_hash: Hash,
    command_string: &str,
    cache_folder: CacheFolder,
    config: &Config,
    project: Option<&Project>,
    root_folder: PathBuf,
    filtered_env: BTreeMap<String, String>,
) -> anyhow::Result<()> {
    info!(
        "Cash miss for {} [{}]",
        command_hash.to_hex(),
        command_string
    );
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

    let (hash, size) = CachedFile::hash_path(&temp_file_path)?;
    let command_line_output_hash =
        CachedFile::create(cache_folder.root.clone(), temp_file_path, hash, size).await?;

    let output_files = if let Some(project) = project {
        project
            .gather_output_files(&root_folder, &cache_folder)
            .await?
    } else {
        vec![]
    };
    let cached_command = CachedCommand {
        command_line: command_string.into(),
        created: Utc::now(),
        env: filtered_env.into(),
        hash: command_hash.to_string().into(),
        last_accessed: Utc::now(),
        log: command_line_output_hash.to_string().into(),
        output_files,
    };
    cached_command.store_in_cache(&cache_folder.root).await?;
    Ok(())
}

async fn handle_command(
    command_string: &str,
    all_input_paths: Vec<PathBuf>,
    root_folder: PathBuf,
    cache_folder_path: PathBuf,
    config: &Config,
    project: Option<&Project>,
) -> anyhow::Result<()> {
    let env_vars = env::vars().collect::<BTreeMap<String, String>>();
    let filtered_env = config.filter_env_vars(&env_vars, &root_folder)?;
    debug!("Filtered env: {:?}", &filtered_env);

    let command_hash = CachedCommand::create_hash(&command_string, all_input_paths, &filtered_env)?;
    debug!(
        "Computed command hash {} for '{}'",
        command_hash.to_string(),
        command_string
    );
    let cache_folder = CacheFolder::new(cache_folder_path);
    if cache_folder.has_cached_command(&command_hash) {
        handle_existing_command(command_hash, command_string, cache_folder).await?;
    } else {
        handle_new_command(
            command_hash,
            command_string,
            cache_folder,
            config,
            project,
            root_folder,
            filtered_env,
        )
        .await?;
    }
    Ok(())
}

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

    let (config, root_path, cache_folder_path) = initialize(&cli).await?;
    if cli.clear {
        info!("Clearing cache folder {:?}", &cache_folder_path);
        fs::remove_dir_all(&cache_folder_path)?;
    }

    let working_dir_project = config.get_project_for_cwd(&root_path)?;
    let cli_project = cli.project.map(|name| config.get_project(&name)).flatten();
    let project = cli_project.or(working_dir_project);
    let inputs = if let Some(project) = project {
        info!("Operating in project {}", project.name);
        config.get_all_depenend_file_globs(&project)?
    } else {
        vec![]
    };

    let all_paths = get_paths_from_globs(&inputs, &root_path)
        .into_iter()
        .unique()
        .collect();

    let command_string = cli.command.join(" ");
    if command_string.trim().is_empty() {
        debug!("Empty command, don't process");
    } else {
        handle_command(
            &command_string,
            all_paths,
            root_path,
            cache_folder_path,
            &config,
            project,
        )
        .await?;
    }
    Ok(())
}
