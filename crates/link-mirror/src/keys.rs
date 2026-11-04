//! GDK keys to what `MirrorInput` takes: plain characters as text, everything else as an Android key code and meta
//! state.

use gtk4::gdk;
use gtk4::glib::translate::IntoGlib as _;

pub const VOLUME_UP: i32 = 24;
pub const VOLUME_DOWN: i32 = 25;
pub const POWER: i32 = 26;

const META_SHIFT: i32 = 0x1 | 0x40;
const META_ALT: i32 = 0x2 | 0x10;
const META_CTRL: i32 = 0x1000 | 0x2000;
const META_META: i32 = 0x1_0000 | 0x2_0000;

#[derive(Debug, PartialEq)]
pub enum Key {
    Text(String),
    Android { code: i32, meta: i32 },
}

/// What a key press sends; None for keys the phone has no use for.
pub fn map(key: gdk::Key, state: gdk::ModifierType) -> Option<Key> {
    let chord =
        state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK);
    if !chord && let Some(c) = key.to_unicode().filter(|c| !c.is_control()) {
        return Some(Key::Text(c.to_string()));
    }
    let code = code(key)?;
    let meta = [
        (gdk::ModifierType::SHIFT_MASK, META_SHIFT),
        (gdk::ModifierType::ALT_MASK, META_ALT),
        (gdk::ModifierType::CONTROL_MASK, META_CTRL),
        (gdk::ModifierType::SUPER_MASK, META_META),
    ]
    .into_iter()
    .filter(|(mask, _)| state.contains(*mask))
    .fold(0, |meta, (_, bits)| meta | bits);
    Some(Key::Android { code, meta })
}

fn code(key: gdk::Key) -> Option<i32> {
    use gdk::Key as K;
    let offset = |from: K, to: K, base: i32| {
        let (key, from, to) = (key.to_lower().into_glib(), from.into_glib(), to.into_glib());
        (from..=to).contains(&key).then(|| base + i32::try_from(key - from).unwrap_or(0))
    };
    if let Some(code) =
        offset(K::a, K::z, 29).or_else(|| offset(K::_0, K::_9, 7)).or_else(|| offset(K::F1, K::F12, 131))
    {
        return Some(code);
    }
    Some(match key {
        K::Escape => 4,
        K::Up | K::KP_Up => 19,
        K::Down | K::KP_Down => 20,
        K::Left | K::KP_Left => 21,
        K::Right | K::KP_Right => 22,
        K::comma => 55,
        K::period => 56,
        K::Tab | K::ISO_Left_Tab => 61,
        K::space => 62,
        K::Return | K::KP_Enter => 66,
        K::BackSpace => 67,
        K::minus => 69,
        K::equal => 70,
        K::slash => 76,
        K::Menu => 82,
        K::Page_Up | K::KP_Page_Up => 92,
        K::Page_Down | K::KP_Page_Down => 93,
        K::Delete | K::KP_Delete => 112,
        K::Home | K::KP_Home => 122,
        K::End | K::KP_End => 123,
        K::Insert => 124,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_keys() {
        use gdk::{Key as K, ModifierType as M};
        let android = |code, meta| Some(Key::Android { code, meta });
        for (key, state, expected) in [
            (K::c, M::CONTROL_MASK, android(31, META_CTRL)),
            (K::V, M::CONTROL_MASK | M::SHIFT_MASK, android(50, META_CTRL | META_SHIFT)),
            (K::Left, M::empty(), android(21, 0)),
            (K::Down, M::SHIFT_MASK, android(20, META_SHIFT)),
            (K::ISO_Left_Tab, M::SHIFT_MASK, android(61, META_SHIFT)),
            (K::Escape, M::empty(), android(4, 0)),
            (K::Return, M::empty(), android(66, 0)),
            (K::_7, M::ALT_MASK, android(14, META_ALT)),
            (K::A, M::SHIFT_MASK, Some(Key::Text("A".into()))),
            (K::eacute, M::empty(), Some(Key::Text("é".into()))),
            (K::Shift_L, M::empty(), None),
        ] {
            assert_eq!(map(key, state), expected, "{key:?} {state:?}");
        }
    }
}
