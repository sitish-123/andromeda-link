//! Remote control, the desktop's screen on a phone: the pump from `link-session`'s socket of access units, in the
//! mirror stream's framing, into a `mirror-data` stream to the phone. A switch to another source replaces the pump; the
//! socket's end ends only the stream, and `StopRemoteControl` or the session leaving ends remote control.

use std::collections::HashMap;
use std::os::fd::OwnedFd;

use link_core::identity::DeviceId;
use link_core::mirror::{MirrorSender, Unit};
use link_core::proto::message::MAX_MIRROR_UNIT;
use tokio::io::AsyncReadExt as _;
use tokio::task::{AbortHandle, JoinSet};

#[derive(Default)]
pub struct Remotes {
    running: HashMap<DeviceId, AbortHandle>,
    pumps: JoinSet<DeviceId>,
}

impl Remotes {
    pub fn running(&self, id: &DeviceId) -> bool {
        self.running.contains_key(id)
    }

    /// Stops every phone's pump and returns the phones.
    pub fn stop_all(&mut self) -> Vec<DeviceId> {
        let ids: Vec<DeviceId> = self.running.keys().cloned().collect();
        for id in &ids {
            self.stop(id);
        }
        ids
    }

    /// Pumps `socket` into `sender`, after ending the phone's previous stream.
    pub fn serve(&mut self, id: DeviceId, socket: OwnedFd, sender: MirrorSender) -> std::io::Result<()> {
        let socket = std::os::unix::net::UnixStream::from(socket);
        socket.set_nonblocking(true)?;
        let socket = tokio::net::UnixStream::from_std(socket)?;
        self.stop(&id);
        let pumped = id.clone();
        let pump = self.pumps.spawn(async move {
            if let Err(error) = pump(socket, sender).await {
                log::info!("{pumped}: remote control stream ended: {error}");
            }
            pumped
        });
        self.running.insert(id, pump);
        Ok(())
    }

    pub fn stop(&mut self, id: &DeviceId) {
        if let Some(pump) = self.running.remove(id) {
            pump.abort();
        }
    }

    /// Reaps the next pump to end, with its phone, which still counts as running until stopped.
    pub async fn ended(&mut self) -> Option<DeviceId> {
        loop {
            match self.pumps.join_next().await? {
                Ok(id) => return Some(id),
                Err(join) if join.is_cancelled() => {}
                Err(join) => log::warn!("remote control pump: {join}"),
            }
        }
    }
}

async fn pump(mut socket: tokio::net::UnixStream, mut sender: MirrorSender) -> anyhow::Result<()> {
    let mut header = [0; 13];
    loop {
        match socket.read_exact(&mut header).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        }
        let [a, b, c, d, pts @ .., flags] = header;
        let len = u32::from_be_bytes([a, b, c, d]) as usize;
        anyhow::ensure!(len <= MAX_MIRROR_UNIT, "an access unit over 4 MiB");
        let mut data = vec![0; len];
        socket.read_exact(&mut data).await?;
        sender.send(&Unit { pts_us: u64::from_be_bytes(pts), flags, data }).await?;
    }
    sender.finish();
    Ok(())
}
