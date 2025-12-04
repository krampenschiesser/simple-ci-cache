use std::path::PathBuf;

use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, stdout},
    process::ChildStdout,
};
use tracing::error;

pub async fn redirect_to_file_and_stdout(
    temp_file_path: PathBuf,
    mut child_stdout: ChildStdout,
) -> anyhow::Result<()> {
    let mut temp_output = File::create(temp_file_path).await?;
    let mut current_stdout = stdout();
    let mut buf = vec![0; 1024];

    loop {
        match child_stdout.read(&mut buf).await {
            // Return value of `Ok(0)` signifies that the remote has
            // closed
            Ok(0) => break,
            Ok(n) => {
                temp_output.write_all(&buf[..n]).await?;
                current_stdout.write_all(&buf[..n]).await?;
            }
            Err(e) => {
                error!("Encountered error while reading child stdout: {}", e);
                break;
            }
        }
    }
    temp_output.flush().await?;
    Ok(())
}
