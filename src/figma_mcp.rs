use eyre::WrapErr;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::FigmaConfig;

pub struct FigmaMcpProcess {
    child: Child,
}

impl FigmaMcpProcess {
    pub async fn start(config: &FigmaConfig) -> eyre::Result<Self> {
        tracing::info!(
            host = %config.host,
            port = config.port,
            "starting figma-developer-mcp HTTP server"
        );

        let mut child = Command::new("npx")
            .args([
                "-y",
                "figma-developer-mcp",
                "--port",
                &config.port.to_string(),
                "--host",
                &config.host,
                "--figma-api-key",
                &config.access_token,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| "spawning figma-developer-mcp process")?;

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(log_child_stream("figma-mcp::stderr", stderr));
        }
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(log_child_stream("figma-mcp::stdout", stdout));
        }

        tracing::info!("figma-developer-mcp process spawned");

        Ok(Self { child })
    }

    pub async fn shutdown(&mut self) -> eyre::Result<()> {
        tracing::info!("sending SIGTERM to figma-developer-mcp");
        self.child
            .kill()
            .await
            .with_context(|| "killing figma-developer-mcp process")?;
        self.child
            .wait()
            .await
            .with_context(|| "waiting for figma-developer-mcp to exit")?;
        tracing::info!("figma-developer-mcp process stopped");
        Ok(())
    }
}

async fn log_child_stream(
    label: &'static str,
    stream: impl tokio::io::AsyncRead + Unpin,
) {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::info!(target = label, "{}", line);
    }
}
