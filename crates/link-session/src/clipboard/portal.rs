//! The clipboard through the Clipboard portal on a `RemoteDesktop` session (GNOME, KDE).

use std::collections::HashMap;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use zbus::proxy::SignalStream;
use zbus::{OwnedFd, Variant};
use zbus::{OwnedObjectPath, OwnedVariant};

use super::{Event, REMOTE_MIME, plan, read_fd};
use crate::portal::{CLIPBOARD, Session};

pub struct Portal {
    proxy: zbus::Proxy<'static>,
    session: OwnedObjectPath,
    events: UnboundedReceiver<Event>,
}

/// Turns the portal's signals into events in a task of its own, so no work is lost when a caller drops `next`.
struct Watcher {
    proxy: zbus::Proxy<'static>,
    session: OwnedObjectPath,
    owner_changed: SignalStream<'static>,
    transfers: SignalStream<'static>,
}

impl Portal {
    /// `None` when the session has no clipboard.
    pub async fn connect(session: &Session) -> anyhow::Result<Option<Self>> {
        if !session.clipboard {
            return Ok(None);
        }
        let proxy = zbus::Proxy::new_owned(
            session.bus.clone(),
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            CLIPBOARD,
        )
        .await?;
        let owner_changed = proxy.receive_signal("SelectionOwnerChanged").await?;
        let transfers = proxy.receive_signal("SelectionTransfer").await?;
        let session = session.handle.clone();
        let mut watcher = Watcher { proxy: proxy.clone(), session: session.clone(), owner_changed, transfers };
        let (sender, events) = unbounded_channel();
        tokio::spawn(async move {
            loop {
                match watcher.next().await {
                    Ok(event) => {
                        if sender.send(event).is_err() {
                            return;
                        }
                    }
                    Err(error) => return log::warn!("the Clipboard portal stopped: {error:#}"),
                }
            }
        });
        Ok(Some(Self { proxy, session, events }))
    }

    pub async fn next(&mut self) -> anyhow::Result<Event> {
        self.events.recv().await.context("the Clipboard portal stopped")
    }

    pub async fn offer_remote(&mut self, mimes: &[String]) -> anyhow::Result<()> {
        let mimes: Vec<String> = mimes.iter().cloned().chain([REMOTE_MIME.to_owned()]).collect();
        let options = HashMap::from([("mime_types", Variant::from(mimes))]);
        let () = self.proxy.call("SetSelection", &(&self.session, options)).await?;
        Ok(())
    }
}

impl Watcher {
    async fn next(&mut self) -> anyhow::Result<Event> {
        loop {
            tokio::select! {
                Some(message) = self.owner_changed.next() => {
                    let (session, options): (OwnedObjectPath, HashMap<String, OwnedVariant>) = message.body().deserialize()?;
                    if session != self.session {
                        continue;
                    }
                    let ours = options.get("session_is_owner").and_then(|value| bool::try_from(value).ok());
                    let mimes: Vec<String> =
                        options.get("mime_types").and_then(|value| value.try_clone().ok()?.try_into().ok()).unwrap_or_default();
                    if ours == Some(true) {
                        continue;
                    }
                    if let Some(event) = self.read(&mimes).await {
                        return Ok(event);
                    }
                }
                Some(message) = self.transfers.next() => {
                    let (session, mime, serial): (OwnedObjectPath, String, u32) = message.body().deserialize()?;
                    if session != self.session {
                        continue;
                    }
                    let fd: OwnedFd = self.proxy.call("SelectionWrite", &(&self.session, serial)).await?;
                    let (done, finished) = tokio::sync::oneshot::channel();
                    let proxy = self.proxy.clone();
                    let session = self.session.clone();
                    tokio::spawn(async move {
                        let success = finished.await.unwrap_or(false);
                        let result: zbus::Result<()> =
                            proxy.call("SelectionWriteDone", &(&session, serial, success)).await;
                        if let Err(error) = result {
                            log::warn!("SelectionWriteDone: {error}");
                        }
                    });
                    return Ok(Event::Paste { mime, sink: fd.into(), done: Some(done) });
                }
                else => anyhow::bail!("the Clipboard portal's signals ended"),
            }
        }
    }

    async fn read(&self, mimes: &[String]) -> Option<Event> {
        let mut types = Vec::new();
        for names in plan(mimes) {
            let read = async {
                let fd: OwnedFd = self.proxy.call("SelectionRead", &(&self.session, &names[0])).await?;
                read_fd(fd.into()).await.context("reading the selection")
            };
            match read.await {
                Ok(data) if !data.is_empty() => types.push((names, data)),
                Ok(_) => {}
                Err(error) => log::warn!("{error:#}"),
            }
        }
        (!types.is_empty()).then_some(Event::Local { types })
    }
}
