//! Phone notifications through `UNUserNotificationCenter`, with their actions and inline reply. The center works only
//! in an app bundle whose main bundle has an identifier, signed (ad hoc at least); outside one there are no
//! notifications. macOS groups them, folds old ones and hides their text on the lock screen by its own settings.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationDismissActionIdentifier, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSound, UNTextInputNotificationAction, UNTextInputNotificationResponse,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use tokio::sync::mpsc;

use crate::link::Link;

const ACTION_PREFIX: &str = "phone:";
const REPLY_PREFIX: &str = "reply:";
/// Between the device id and the notification key in a request identifier; neither holds it.
const SEPARATOR: char = '\u{1f}';

/// A user's answer to a phone notification; `action` is None for a dismissal.
struct Response {
    device: String,
    id: String,
    action: Option<String>,
    reply: String,
}

define_class!(
    // SAFETY: NSObject has no subclassing rules, and `Delegate` adds no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AndromedaLinkNotificationDelegate"]
    #[ivars = mpsc::UnboundedSender<Response>]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shows phone notifications even when Link's app is in front.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            let options = UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound;
            handler.call((options,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            handler: &block2::DynBlock<dyn Fn()>,
        ) {
            if let Some(response) = parse(response) {
                drop(self.ivars().send(response));
            }
            handler.call(());
        }
    }
);

impl Delegate {
    fn new(responses: mpsc::UnboundedSender<Response>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(responses);
        // SAFETY: NSObject's init takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

/// The phone's (device, key, action, reply) a response answers; None for a click on the notification itself.
fn parse(response: &UNNotificationResponse) -> Option<Response> {
    let identifier = response.notification().request().identifier().to_string();
    let (device, id) = identifier.split_once(SEPARATOR)?;
    let action = response.actionIdentifier();
    // SAFETY: a framework constant, set before any response arrives.
    let action = if &*action == unsafe { UNNotificationDismissActionIdentifier } {
        None
    } else {
        let action = action.to_string();
        let action = action.strip_prefix(ACTION_PREFIX).or_else(|| action.strip_prefix(REPLY_PREFIX))?.to_owned();
        Some(action)
    };
    let reply = response
        .downcast_ref::<UNTextInputNotificationResponse>()
        .map(|reply| reply.userText().to_string())
        .unwrap_or_default();
    Some(Response { device: device.to_owned(), id: id.to_owned(), action, reply })
}

pub struct Notifications {
    center: Retained<UNUserNotificationCenter>,
    /// The center holds its delegate weakly.
    _delegate: Retained<Delegate>,
    /// A category per set of actions, as the center takes actions only through registered categories.
    categories: HashMap<String, Retained<UNNotificationCategory>>,
}

impl Notifications {
    /// None outside an app bundle, where the center would raise an exception. The first run asks the user.
    pub fn connect(link: Link) -> Option<Self> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            log::warn!("notifications need link-session to run from Link's signed app bundle");
            return None;
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (responses, mut answers) = mpsc::unbounded_channel();
        let delegate = Delegate::new(responses);
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let granted = RcBlock::new(|granted: Bool, _error: *mut NSError| {
            if !granted.as_bool() {
                log::warn!("the user has not allowed Link's notifications");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &granted,
        );
        tokio::spawn(async move {
            while let Some(answer) = answers.recv().await {
                match answer.action {
                    Some(action) => link.notification_action(&answer.device, &answer.id, &action, &answer.reply).await,
                    None => link.notification_dismiss(&answer.device, &answer.id).await,
                }
            }
        });
        Some(Self { center, _delegate: delegate, categories: HashMap::new() })
    }

    /// Shows a phone notification, or replaces the one with its key; an update without `alert` makes no sound.
    pub fn posted(&mut self, posted: &link_api::NotificationPostedArgs<'_>) {
        let content = UNMutableNotificationContent::new();
        let (title, subtitle) = if posted.title.is_empty() { (posted.app, "") } else { (posted.title, posted.app) };
        content.setTitle(&NSString::from_str(title));
        content.setSubtitle(&NSString::from_str(subtitle));
        content.setBody(&NSString::from_str(posted.text));
        content.setThreadIdentifier(&NSString::from_str(&format!("{}/{}", posted.device_id, posted.app)));
        content.setCategoryIdentifier(&NSString::from_str(&self.category(&posted.actions)));
        if posted.alert {
            content.setSound(Some(&UNNotificationSound::defaultSound()));
        }
        let identifier = NSString::from_str(&format!("{}{SEPARATOR}{}", posted.device_id, posted.id));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
        let added = RcBlock::new(|error: *mut NSError| {
            // SAFETY: the center passes a valid error or null.
            if let Some(error) = unsafe { error.as_ref() } {
                log::warn!("showing a phone notification: {}", error.localizedDescription());
            }
        });
        self.center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
    }

    pub fn removed(&self, device: &str, id: &str) {
        let identifiers = NSArray::from_retained_slice(&[NSString::from_str(&format!("{device}{SEPARATOR}{id}"))]);
        self.center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
        self.center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
    }

    /// The category for `actions` (id, label, takes a reply), registered when new. Its custom dismiss action tells the
    /// phone when the user clears the notification.
    fn category(&mut self, actions: &[(&str, &str, bool)]) -> String {
        let mut hasher = DefaultHasher::new();
        actions.hash(&mut hasher);
        let name = format!("link:{:016x}", hasher.finish());
        if self.categories.contains_key(&name) {
            return name;
        }
        let actions: Vec<Retained<UNNotificationAction>> = actions
            .iter()
            .map(|&(id, label, reply)| {
                let title = NSString::from_str(label);
                let none = UNNotificationActionOptions::empty();
                if reply {
                    let id = NSString::from_str(&format!("{REPLY_PREFIX}{id}"));
                    let action = UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                        &id, &title, none, &NSString::from_str("Send"), &NSString::from_str(""),
                    );
                    Retained::into_super(action)
                } else {
                    let id = NSString::from_str(&format!("{ACTION_PREFIX}{id}"));
                    UNNotificationAction::actionWithIdentifier_title_options(&id, &title, none)
                }
            })
            .collect();
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(&name),
            &NSArray::from_retained_slice(&actions),
            &NSArray::new(),
            UNNotificationCategoryOptions::CustomDismissAction,
        );
        self.categories.insert(name.clone(), category);
        let all: Vec<Retained<UNNotificationCategory>> = self.categories.values().cloned().collect();
        self.center.setNotificationCategories(&NSSet::from_retained_slice(&all));
        name
    }
}
