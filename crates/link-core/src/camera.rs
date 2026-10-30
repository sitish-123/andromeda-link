//! The phone taking a photo for a desktop whose browse switch is on: the app's camera, off the actor. The desktop
//! reads the photo through browsing, from the path the answer gives under the Photos root.

use std::sync::Arc;

use link_proto::message::{Message, PhotoRefusal, PhotoTaken};

use crate::browse::PHOTOS;
use crate::session::SessionHandle;

/// Called off the actor; blocks while the user takes the photo.
pub trait Camera: Send + Sync {
    /// Opens the camera for `desktop`, by name; returns the photo as (album, file name) once it is saved.
    fn take(&self, desktop: &str) -> Result<(String, String), PhotoRefusal>;
}

pub fn refused(req: u64, reason: PhotoRefusal) -> Message {
    Message::PhotoTaken(PhotoTaken { req, path: None, refused: Some(reason) })
}

/// Takes one photo and answers `req` on `session`.
pub async fn answer(camera: Arc<dyn Camera>, session: SessionHandle, desktop: String, req: u64) {
    let reply = tokio::task::spawn_blocking(move || match camera.take(&desktop) {
        Ok((album, name)) => {
            let path = format!("/{PHOTOS}/{album}/{name}");
            let taken = Message::PhotoTaken(PhotoTaken { req, path: Some(path), refused: None });
            if taken.validate().is_ok() { taken } else { refused(req, PhotoRefusal::Failed) }
        }
        Err(reason) => refused(req, reason),
    })
    .await;
    let Ok(reply) = reply else { return log::warn!("taking a photo panicked") };
    if let Err(error) = session.send(reply).await {
        log::info!("answering a photo request: {error}");
    }
}
