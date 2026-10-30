//! Keeps the session from going idle while a phone mirrors here or presents (the daemon's `KeepAwake`), with an
//! inhibit on the shell's org.freedesktop.ScreenSaver; the shell drops it too if this process exits.

const SERVICE: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";

pub struct Awake {
    proxy: zbus::Proxy<'static>,
    cookie: Option<u32>,
}

impl Awake {
    pub async fn connect(bus: &zbus::Connection) -> zbus::Result<Self> {
        Ok(Self { proxy: zbus::Proxy::new_owned(bus.clone(), SERVICE, PATH, SERVICE).await?, cookie: None })
    }

    pub async fn set(&mut self, on: bool) {
        match (on, self.cookie) {
            (true, None) => {
                match self.proxy.call("Inhibit", &("Andromeda Link", "A phone shows its screen or presents")).await {
                    Ok(cookie) => {
                        log::info!("keeping the session awake");
                        self.cookie = Some(cookie);
                    }
                    Err(error) => log::warn!("inhibiting idle: {error}"),
                }
            }
            (false, Some(cookie)) => {
                self.cookie = None;
                match self.proxy.call::<_, _, ()>("UnInhibit", &(cookie,)).await {
                    Ok(()) => log::info!("the session may go idle again"),
                    Err(error) => log::warn!("releasing the idle inhibit: {error}"),
                }
            }
            _ => {}
        }
    }
}
