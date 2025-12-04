# Intro

caching utility for the command line, aimed at CI environments.
It runs a command and then saves and compresses the log-output (stdout+stderr) and associated files in a local cache folder.
When the command, its input files or env vars change, the output will be considered stale and the command is re-run.

This cache is not atomic, so if errors are encountered, delete the cache.

## Compression

Files and outputs are compressed when needed depening on the file extension (only!).
Text like files will be compressed with brotli.
Already compressed files will not undergo any changes.
Other files will be deflated.

# Cache config

You can configure the cache and create projects to handle dependencies.
Please ensure that the dependencies form a directed graph.

## Projects
Each project takes the following configuration input:

```yaml
name: Uniquename
inputs: # glob patterns for input files
    - src/**
outputs: # glob patterns for output files
    - dist/**
env: # environment variables to consider
    - NODE_ENV
    - MAVEN_OPTS
depends_on: 
    - MyOtherProjects
```

To identify a cached command the filtered `env` variables and `inputs` files are hashed together with the command to be executed.
From the projects listed in `depends_on` all inputs are combined to this projects inputs `inputs`.
And of course, a config change will invalidate the whole cache.

# Cache folder

The .cache folder contains the (maybe) compressed files and cached commands linking to those files.

.cache/
.cache/files/asd12xxx
.cache/files/asd12xxx/file.json
.cache/files/asd12xxx/compressed
.cache/commands
.cache/commands/sdf895a/command.json

## file.json

{
    created: DateTime
    original_hash: HashSum
    compression_algorithm: brotli,none,zlib
    original_path: String
}

## command.json

{
    ran: DateTime,
    env: Map,
    command: String,
    hash: Hash
    outputs: Hash[]
    inputs: Hash[]
}

# how to use in CI envs

create your configuration and list your projects and dependencies.
in the ci job,
restore your cache folder before the build, back it up afterwards.
When backing up the folder, it might be useful to `tar` it and not deal with the latency of individual http `HEAD`/`GET` requests.



# Environment variables

CACHE_DIR: overwrites the cache directory to use
CACHE_RO: will not write to the cache, but only use it as read source.

## Future plans

## State

store last file modification, file name, and result hash as additional lookup cache to skip hashing some files.
This will require some rework on the hashsum itself.
Let's see how fast it actually is.

## Remote backends

In the future remote backends will be added. eg. s3.
This will require reworking the caches to some common interfaces so lookup could be local against a folder or remote against a storage provider.

## strace

Use strace to figure out input/output files for an initial configuraiton of a project.
How to convert them into globs?