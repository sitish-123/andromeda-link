//! The Link service on macOS through `IOBluetooth`: the same SDP record as with `BlueZ`, and each incoming RFCOMM
//! channel pumped through a socket pair whose other end goes to the listener, like Android's `RfcommLink`.
//! `IOBluetooth` calls back on the run loop of the thread that registered, so the service runs on a thread of its own.

#![allow(unsafe_code, reason = "FFI to IOBluetooth and CoreFoundation")]

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::net::UnixStream;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{AnyThread as _, DefinedClass as _, Message, define_class, msg_send, sel};
use objc2_core_foundation::{
    CFFileDescriptor, CFFileDescriptorContext, CFOptionFlags, CFRetained, CFRunLoop, kCFFileDescriptorReadCallBack,
    kCFRunLoopDefaultMode,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSNumber, NSString};
use objc2_io_bluetooth::{
    IOBluetoothHostController, IOBluetoothRFCOMMChannel, IOBluetoothRFCOMMChannelDelegate, IOBluetoothSDPServiceRecord,
    IOBluetoothUserNotification, IOBluetoothUserNotificationChannelDirection,
};
use tokio::sync::{mpsc, oneshot};

use super::{CHANNEL, SERVICE_UUID};

/// Publishes the service and returns the adapter's address, formatted as `BlueZ` does.
pub async fn register(incoming: mpsc::Sender<OwnedFd>) -> Result<String, String> {
    let (ready, address) = oneshot::channel();
    std::thread::Builder::new()
        .name("bluetooth".to_owned())
        .spawn(move || serve(incoming, ready))
        .map_err(|error| error.to_string())?;
    address.await.map_err(|_| "the Bluetooth thread ended".to_owned())?
}

fn serve(incoming: mpsc::Sender<OwnedFd>, ready: oneshot::Sender<Result<String, String>>) {
    let service = match publish(incoming) {
        Ok(service) => service,
        Err(error) => {
            drop(ready.send(Err(error)));
            return;
        }
    };
    drop(ready.send(Ok(service.address.clone())));
    CFRunLoop::run();
    log::warn!("Bluetooth: the run loop stopped; no more connections arrive");
    drop(service);
}

/// What keeps the service published and listening.
struct Service {
    address: String,
    _record: Retained<IOBluetoothSDPServiceRecord>,
    _notification: Retained<IOBluetoothUserNotification>,
    _listener: Retained<Listener>,
}

fn publish(incoming: mpsc::Sender<OwnedFd>) -> Result<Service, String> {
    // SAFETY: every IOBluetooth object here is used on this thread only.
    let controller = unsafe { IOBluetoothHostController::defaultController() }.ok_or("no Bluetooth adapter")?;
    // SAFETY: as above.
    let address = unsafe { controller.addressAsString() }.ok_or("Bluetooth is off")?;
    let address = address.to_string().replace('-', ":").to_ascii_uppercase();
    let dictionary = service_dictionary()?;
    // SAFETY: the dictionary is in the format IOBluetooth documents for service records.
    let record = unsafe { IOBluetoothSDPServiceRecord::publishedServiceRecordWithDictionary(Some(&dictionary)) }
        .ok_or("publishing the SDP record failed")?;
    let mut channel = 0;
    // SAFETY: `channel` receives the record's RFCOMM channel.
    let status = unsafe { record.getRFCOMMChannelID(&raw mut channel) };
    if status != 0 {
        return Err(format!("the SDP record has no RFCOMM channel: IOReturn {status:#x}"));
    }
    if u16::from(channel) != CHANNEL {
        log::info!("Bluetooth: macOS gave the service RFCOMM channel {channel}");
    }
    let listener = Listener::new(incoming);
    let target: &AnyObject = &listener;
    // SAFETY: `listener` answers the selector with the documented arguments and lives as long as the notification.
    let notification = unsafe {
        IOBluetoothRFCOMMChannel::registerForChannelOpenNotifications_selector_withChannelID_direction(
            Some(target),
            Some(sel!(channelOpened:channel:)),
            channel,
            IOBluetoothUserNotificationChannelDirection::Incoming,
        )
    }
    .ok_or("registering for RFCOMM connections failed")?;
    Ok(Service { address, _record: record, _notification: notification, _listener: listener })
}

/// The record `BlueZ` gets as XML, in `IOBluetooth`'s dictionary form: attribute ids as keys, UUIDs as data, arrays
/// as sequences, and the channel as a 1-byte unsigned data element.
fn service_dictionary() -> Result<Retained<NSDictionary>, String> {
    let uuid = hex::decode(SERVICE_UUID.replace('-', "")).map_err(|error| error.to_string())?;
    let uuid16 = |value: u16| any(NSData::with_bytes(&value.to_be_bytes()));
    let array = |items: &[Retained<AnyObject>]| any(NSArray::from_retained_slice(items));
    let channel = NSDictionary::<NSString, AnyObject>::from_retained_objects(
        &[
            &*NSString::from_str("DataElementType"),
            &NSString::from_str("DataElementSize"),
            &NSString::from_str("DataElementValue"),
        ],
        &[any(NSNumber::new_u8(1)), any(NSNumber::new_u8(1)), any(NSNumber::new_u16(CHANNEL))],
    );
    let keys = [
        NSString::from_str("0001 - ServiceClassIDList"),
        NSString::from_str("0004 - ProtocolDescriptorList"),
        NSString::from_str("0005 - BrowseGroupList*"),
        NSString::from_str("0100 - ServiceName*"),
    ];
    let values = [
        array(&[any(NSData::with_bytes(&uuid))]),
        // L2CAP, then RFCOMM on the channel.
        array(&[array(&[uuid16(0x0100)]), array(&[uuid16(0x0003), any(channel)])]),
        // The public browse group, which Android's SDP search starts from.
        array(&[uuid16(0x1002)]),
        any(NSString::from_str("Andromeda Link")),
    ];
    let keys: Vec<&NSString> = keys.iter().map(|key| &**key).collect();
    let dictionary = NSDictionary::<NSString, AnyObject>::from_retained_objects(&keys, &values);
    // SAFETY: an untyped dictionary is the same object.
    Ok(unsafe { Retained::cast_unchecked(dictionary) })
}

fn any<T: Message>(object: Retained<T>) -> Retained<AnyObject> {
    // SAFETY: every Objective-C object is an `AnyObject`.
    unsafe { Retained::cast_unchecked(object) }
}

/// Connections opened on the service, and the pumps of those still running.
struct Accept {
    incoming: mpsc::Sender<OwnedFd>,
    /// A closed pump is dropped at the next connection, since it closes from inside its own callbacks.
    pumps: RefCell<Vec<Retained<Pump>>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing rules, and `Listener` adds no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AndromedaLinkRfcommListener"]
    #[ivars = Accept]
    struct Listener;

    unsafe impl NSObjectProtocol for Listener {}

    impl Listener {
        #[unsafe(method(channelOpened:channel:))]
        fn channel_opened(&self, _notification: &IOBluetoothUserNotification, channel: &IOBluetoothRFCOMMChannel) {
            self.open(channel);
        }
    }
);

impl Listener {
    fn new(incoming: mpsc::Sender<OwnedFd>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Accept { incoming, pumps: RefCell::new(Vec::new()) });
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }

    fn open(&self, channel: &IOBluetoothRFCOMMChannel) {
        log::info!("Bluetooth connection on RFCOMM");
        let accept = self.ivars();
        accept.pumps.borrow_mut().retain(|pump| !pump.ivars().closed.get());
        let (ours, theirs) = match UnixStream::pair() {
            Ok(pair) => pair,
            Err(error) => {
                log::warn!("Bluetooth: a socket pair: {error}");
                // SAFETY: the channel is open and used on this thread only.
                unsafe { channel.closeChannel() };
                return;
            }
        };
        // The phone's data is written without waiting only while the pair can hold every stream's credit window.
        if let Err(error) = hold_windows(&ours, &theirs) {
            log::warn!("Bluetooth: sizing the socket pair: {error}");
        }
        let pump = Pump::new(channel, ours);
        let delegate: &AnyObject = &pump;
        // SAFETY: `pump` stays in `pumps` while the channel can call it.
        let status = unsafe { channel.setDelegate(Some(delegate)) };
        if status != 0 {
            log::warn!("Bluetooth: taking the channel: IOReturn {status:#x}");
            return pump.finish();
        }
        if !pump.watch() {
            return pump.finish();
        }
        if accept.incoming.try_send(theirs.into()).is_err() {
            log::warn!("Bluetooth: too many connections waiting");
            return pump.finish();
        }
        accept.pumps.borrow_mut().push(pump);
    }
}

/// One RFCOMM channel and the socket the daemon's end of the pair talks to.
struct Pipe {
    channel: Retained<IOBluetoothRFCOMMChannel>,
    socket: UnixStream,
    readable: RefCell<Option<CFRetained<CFFileDescriptor>>>,
    closed: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing rules, and `Pump` adds no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AndromedaLinkRfcommPump"]
    #[ivars = Pipe]
    struct Pump;

    unsafe impl NSObjectProtocol for Pump {}

    unsafe impl IOBluetoothRFCOMMChannelDelegate for Pump {
        #[unsafe(method(rfcommChannelData:data:length:))]
        fn channel_data(&self, _channel: Option<&IOBluetoothRFCOMMChannel>, data: *mut c_void, length: usize) {
            if length == 0 {
                return;
            }
            // SAFETY: IOBluetooth hands over `length` readable bytes for the duration of the call.
            let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) };
            // A blocking write: the mux's credit windows bound what the phone sends ahead of the daemon.
            if let Err(error) = (&self.ivars().socket).write_all(bytes) {
                log::info!("Bluetooth: passing on the phone's data: {error}");
                self.finish();
            }
        }

        #[unsafe(method(rfcommChannelClosed:))]
        fn channel_closed(&self, _channel: Option<&IOBluetoothRFCOMMChannel>) {
            log::info!("Bluetooth: the RFCOMM channel closed");
            self.finish();
        }
    }
);

impl Pump {
    fn new(channel: &IOBluetoothRFCOMMChannel, socket: UnixStream) -> Retained<Self> {
        let pipe = Pipe { channel: channel.retain(), socket, readable: RefCell::new(None), closed: Cell::new(false) };
        let this = Self::alloc().set_ivars(pipe);
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }

    /// Calls `forward` on this run loop whenever the daemon has written to its end.
    fn watch(&self) -> bool {
        let pipe = self.ivars();
        let context = CFFileDescriptorContext {
            version: 0,
            info: std::ptr::from_ref(self).cast_mut().cast(),
            retain: None,
            release: None,
            copyDescription: None,
        };
        // SAFETY: the context points at this pump, which outlives the descriptor: `finish` invalidates it first.
        let Some(descriptor) = (unsafe {
            CFFileDescriptor::new(None, pipe.socket.as_raw_fd(), false, Some(readable), &raw const context)
        }) else {
            log::warn!("Bluetooth: watching the socket failed");
            return false;
        };
        let Some(source) = CFFileDescriptor::new_run_loop_source(None, Some(&descriptor), 0) else {
            log::warn!("Bluetooth: watching the socket failed");
            return false;
        };
        let Some(run_loop) = CFRunLoop::current() else { return false };
        // SAFETY: CoreFoundation's constant mode name.
        run_loop.add_source(Some(&source), unsafe { kCFRunLoopDefaultMode });
        descriptor.enable_call_backs(kCFFileDescriptorReadCallBack);
        *pipe.readable.borrow_mut() = Some(descriptor);
        true
    }

    /// Sends what the daemon wrote to the phone, one RFCOMM frame at a time.
    fn forward(&self) {
        let pipe = self.ivars();
        // SAFETY: the channel is used on this thread only.
        let mtu = usize::from(unsafe { pipe.channel.getMTU() }).max(1);
        let mut buffer = vec![0; mtu];
        match (&pipe.socket).read(&mut buffer) {
            Ok(0) => return self.finish(),
            Ok(read) => {
                // SAFETY: `buffer` holds `read` bytes, at most the MTU, which is a u16.
                let status =
                    unsafe { pipe.channel.writeSync_length(buffer.as_mut_ptr().cast(), read.try_into().unwrap_or(0)) };
                if status != 0 {
                    log::info!("Bluetooth: sending to the phone: IOReturn {status:#x}");
                    return self.finish();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                log::info!("Bluetooth: reading the daemon's data: {error}");
                return self.finish();
            }
        }
        if let Some(descriptor) = pipe.readable.borrow().as_ref() {
            descriptor.enable_call_backs(kCFFileDescriptorReadCallBack);
        }
    }

    /// Closes both ends once; the daemon sees the socket end.
    fn finish(&self) {
        let pipe = self.ivars();
        if pipe.closed.replace(true) {
            return;
        }
        if let Some(descriptor) = pipe.readable.borrow_mut().take() {
            descriptor.invalidate();
        }
        drop(pipe.socket.shutdown(std::net::Shutdown::Both));
        // SAFETY: the channel is used on this thread only; closing a closed channel only fails.
        unsafe {
            pipe.channel.setDelegate(None);
            pipe.channel.closeChannel();
        }
    }
}

unsafe extern "C-unwind" fn readable(_descriptor: *mut CFFileDescriptor, _flags: CFOptionFlags, info: *mut c_void) {
    // SAFETY: `info` is the pump that created the descriptor, alive until `finish` invalidates it.
    let pump = unsafe { &*info.cast::<Pump>() };
    pump.forward();
}

/// Every stream's credit window plus room for control frames: 10 x 256 KiB.
#[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap, reason = "constants far below 2^31")]
const PIPE_BUFFER: libc::c_int =
    ((link_core::proto::mux::MAX_PEER_STREAMS as u32 + 1) * link_core::proto::mux::WINDOW) as libc::c_int;

fn hold_windows(ours: &UnixStream, theirs: &UnixStream) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let size = PIPE_BUFFER;
    for (socket, option) in [(ours, libc::SO_SNDBUF), (theirs, libc::SO_RCVBUF)] {
        // SAFETY: a valid socket and a c_int option value of the size passed.
        let status = unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&raw const size).cast(),
                libc::socklen_t::try_from(std::mem::size_of::<libc::c_int>()).unwrap_or(4),
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
