//! The default output's volume and mute through `WirePlumber` (`wpctl`), watched through `pactl subscribe` on
//! `pipewire-pulse` when it runs.

use anyhow::{Context, bail};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

pub struct Volume {
    /// `pactl subscribe` and its output; None when it cannot run, so only commands from phones update the status.
    changes: Option<(Child, Lines<BufReader<ChildStdout>>)>,
}

impl Volume {
    /// None when `wpctl` cannot read the default output, as without `WirePlumber` or an output.
    pub async fn connect() -> anyhow::Result<Option<Self>> {
        if read().await.is_err() {
            return Ok(None);
        }
        let watcher =
            Command::new("pactl").arg("subscribe").stdout(std::process::Stdio::piped()).kill_on_drop(true).spawn();
        let changes = watcher.ok().and_then(|mut child| {
            let stdout = child.stdout.take()?;
            Some((child, BufReader::new(stdout).lines()))
        });
        Ok(Some(Self { changes }))
    }

    /// (percent, muted) of the default output.
    pub async fn read(&self) -> anyhow::Result<(u8, bool)> {
        read().await
    }

    pub async fn set_volume(&self, percent: u8) -> anyhow::Result<()> {
        wpctl(&["set-volume", SINK, &format!("{}%", percent.min(100))]).await.map(drop)
    }

    pub async fn set_muted(&self, muted: bool) -> anyhow::Result<()> {
        wpctl(&["set-mute", SINK, if muted { "1" } else { "0" }]).await.map(drop)
    }

    /// Waits for the next change to a sink; pending forever without a watcher.
    pub async fn changed(&mut self) -> anyhow::Result<()> {
        let Some((_, changes)) = self.changes.as_mut() else { return std::future::pending().await };
        loop {
            match changes.next_line().await {
                Ok(Some(line)) if line.contains("'change' on sink") => return Ok(()),
                Ok(Some(_)) => {}
                // The watcher is gone; phone commands still report the volume.
                Ok(None) => {
                    self.changes = None;
                    return std::future::pending().await;
                }
                Err(error) => {
                    self.changes = None;
                    return Err(error.into());
                }
            }
        }
    }
}

async fn read() -> anyhow::Result<(u8, bool)> {
    parse(&wpctl(&["get-volume", SINK]).await?)
}

async fn wpctl(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("wpctl").args(args).output().await.context("running wpctl")?;
    if !output.status.success() {
        bail!("wpctl {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `Volume: 0.40` or `Volume: 0.40 [MUTED]`; a volume over 100% reads as 100.
fn parse(text: &str) -> anyhow::Result<(u8, bool)> {
    let level = text
        .split_whitespace()
        .nth(1)
        .and_then(|level| level.parse::<f64>().ok())
        .with_context(|| format!("unexpected wpctl output {text:?}"))?;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=100 first")]
    let percent = (level * 100.0).round().clamp(0.0, 100.0) as u8;
    Ok((percent, text.contains("[MUTED]")))
}
