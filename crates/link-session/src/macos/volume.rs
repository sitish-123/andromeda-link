//! The default output device's volume and mute through `CoreAudio`, watched with property listeners.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]
#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::unused_self,
    reason = "the Linux backend's interface, whose calls wait"
)]

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{bail, ensure};
use objc2_core_audio::{
    AudioObjectAddPropertyListener, AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListener, AudioObjectSetPropertyData, kAudioDevicePropertyMute,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioHardwarePropertyRunLoop, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};
use tokio::sync::mpsc;

/// `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` ('vmvc', in AudioToolbox): one scalar for the device's
/// channels, as the menu bar's slider sets it.
const MAIN_VOLUME: u32 = 0x766d_7663;
const SYSTEM: AudioObjectID = kAudioObjectSystemObject as AudioObjectID;

pub struct Volume {
    /// The device the listeners watch.
    device: AudioObjectID,
    changes: mpsc::UnboundedReceiver<()>,
    /// The listeners' client data; boxed, so its address stays put until they are removed.
    sender: Box<mpsc::UnboundedSender<()>>,
}

impl Volume {
    /// None without a default output device, or when it has no main volume.
    pub async fn connect() -> anyhow::Result<Option<Self>> {
        // `CoreAudio` otherwise sends notifications through the main run loop, which this process does not run.
        let no_run_loop: *const c_void = std::ptr::null();
        drop(set(SYSTEM, address(kAudioHardwarePropertyRunLoop, kAudioObjectPropertyScopeGlobal), no_run_loop));
        let Ok(device) = default_device() else { return Ok(None) };
        if read_device(device).is_err() {
            return Ok(None);
        }
        let (sender, changes) = mpsc::unbounded_channel();
        let volume = Self { device, changes, sender: Box::new(sender) };
        volume.listen(
            SYSTEM,
            &[address(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal)],
            true,
        );
        volume.listen(device, &device_addresses(), true);
        Ok(Some(volume))
    }

    /// (percent, muted) of the default output.
    pub async fn read(&self) -> anyhow::Result<(u8, bool)> {
        read_device(default_device()?)
    }

    pub async fn set_volume(&self, percent: u8) -> anyhow::Result<()> {
        set(
            default_device()?,
            address(MAIN_VOLUME, kAudioObjectPropertyScopeOutput),
            f32::from(percent.min(100)) / 100.0,
        )
    }

    pub async fn set_muted(&self, muted: bool) -> anyhow::Result<()> {
        set(default_device()?, address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput), u32::from(muted))
    }

    /// Waits for the next change to the default output or its volume; a new default device is watched from then on.
    pub async fn changed(&mut self) -> anyhow::Result<()> {
        // Never None: `self.sender` lives as long as the receiver.
        self.changes.recv().await;
        if let Ok(device) = default_device()
            && device != self.device
        {
            self.listen(self.device, &device_addresses(), false);
            self.device = device;
            self.listen(device, &device_addresses(), true);
        }
        Ok(())
    }

    /// Adds or removes the change listener on `object`'s properties.
    fn listen(&self, object: AudioObjectID, addresses: &[AudioObjectPropertyAddress], on: bool) {
        let data: *const mpsc::UnboundedSender<()> = &raw const *self.sender;
        for address in addresses {
            // SAFETY: `data` outlives the listener, which `Drop` removes.
            let status = unsafe {
                if on {
                    AudioObjectAddPropertyListener(
                        object,
                        NonNull::from(address),
                        Some(changed),
                        data.cast_mut().cast(),
                    )
                } else {
                    AudioObjectRemovePropertyListener(
                        object,
                        NonNull::from(address),
                        Some(changed),
                        data.cast_mut().cast(),
                    )
                }
            };
            if status != 0 {
                log::debug!("audio property listener on {object}: {status}");
            }
        }
    }
}

impl Drop for Volume {
    fn drop(&mut self) {
        self.listen(
            SYSTEM,
            &[address(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal)],
            false,
        );
        self.listen(self.device, &device_addresses(), false);
    }
}

/// Runs on a `CoreAudio` thread.
unsafe extern "C-unwind" fn changed(
    _object: AudioObjectID,
    _count: u32,
    _addresses: NonNull<AudioObjectPropertyAddress>,
    data: *mut c_void,
) -> i32 {
    // SAFETY: `data` is the `Volume`'s boxed sender, alive while the listener is.
    let sender = unsafe { &*data.cast::<mpsc::UnboundedSender<()>>() };
    let _ = sender.send(());
    0
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

fn device_addresses() -> [AudioObjectPropertyAddress; 2] {
    [
        address(MAIN_VOLUME, kAudioObjectPropertyScopeOutput),
        address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput),
    ]
}

fn default_device() -> anyhow::Result<AudioObjectID> {
    let device: AudioObjectID =
        get(SYSTEM, address(kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyScopeGlobal))?;
    if device == 0 {
        bail!("no default output device");
    }
    Ok(device)
}

/// A device without a main mute reads as not muted.
fn read_device(device: AudioObjectID) -> anyhow::Result<(u8, bool)> {
    let level: f32 = get(device, address(MAIN_VOLUME, kAudioObjectPropertyScopeOutput))?;
    let muted: u32 = get(device, address(kAudioDevicePropertyMute, kAudioObjectPropertyScopeOutput)).unwrap_or(0);
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=100 first")]
    let percent = (level * 100.0).round().clamp(0.0, 100.0) as u8;
    Ok((percent, muted != 0))
}

fn get<T: Copy + Default>(object: AudioObjectID, address: AudioObjectPropertyAddress) -> anyhow::Result<T> {
    let mut value = T::default();
    let mut size = u32::try_from(size_of::<T>())?;
    // SAFETY: `value` has room for `size` bytes, and the property is of type `T`.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    ensure!(status == 0, "reading audio property {:#x}: {status}", address.mSelector);
    Ok(value)
}

fn set<T: Copy>(object: AudioObjectID, address: AudioObjectPropertyAddress, value: T) -> anyhow::Result<()> {
    // SAFETY: `value` holds `size_of::<T>()` bytes, and the property is of type `T`.
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            u32::try_from(size_of::<T>())?,
            NonNull::from(&value).cast(),
        )
    };
    ensure!(status == 0, "setting audio property {:#x}: {status}", address.mSelector);
    Ok(())
}
