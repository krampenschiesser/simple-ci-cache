use clap::{Parser, arg, command};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct CommandLineArgs {
    #[arg(last = true)]
    pub command: Vec<String>,

    #[arg(short)]
    pub config: Option<String>,

    #[arg(long)]
    #[arg(default_value = "false")]
    pub verbose: bool,

    #[arg(long)]
    #[arg(default_value = "false")]
    pub clear: bool,

    #[arg(short)]
    pub project: Option<String>,
}
