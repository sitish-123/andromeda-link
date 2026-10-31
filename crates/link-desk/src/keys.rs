//! Qt keys to what `MirrorInput` takes: plain characters as text, everything else as an Android key code and meta
//! state.

const SHIFT: i32 = 0x0200_0000;
const CONTROL: i32 = 0x0400_0000;
const ALT: i32 = 0x0800_0000;
const META: i32 = 0x1000_0000;

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
pub fn map(key: i32, modifiers: i32, text: &str) -> Option<Key> {
    if modifiers & (CONTROL | ALT | META) == 0 && text.chars().next().is_some_and(|c| !c.is_control()) {
        return Some(Key::Text(text.to_owned()));
    }
    let code = code(key)?;
    let meta = [(SHIFT, META_SHIFT), (ALT, META_ALT), (CONTROL, META_CTRL), (META, META_META)]
        .into_iter()
        .filter(|(qt, _)| modifiers & qt != 0)
        .fold(0, |meta, (_, bits)| meta | bits);
    Some(Key::Android { code, meta })
}

fn code(key: i32) -> Option<i32> {
    Some(match key {
        0x41..=0x5a => key - 0x41 + 29,
        0x30..=0x39 => key - 0x30 + 7,
        0x0100_0030..=0x0100_003b => key - 0x0100_0030 + 131,
        0x0100_0000 => 4,
        0x0100_0013 => 19,
        0x0100_0015 => 20,
        0x0100_0012 => 21,
        0x0100_0014 => 22,
        0x2c => 55,
        0x2e => 56,
        0x0100_0001 | 0x0100_0002 => 61,
        0x20 => 62,
        0x0100_0004 | 0x0100_0005 => 66,
        0x0100_0003 => 67,
        0x2d => 69,
        0x3d => 70,
        0x2f => 76,
        0x0100_0055 => 82,
        0x0100_0016 => 92,
        0x0100_0017 => 93,
        0x0100_0007 => 112,
        0x0100_0010 => 122,
        0x0100_0011 => 123,
        0x0100_0006 => 124,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_keys() {
        let android = |code, meta| Some(Key::Android { code, meta });
        for (key, modifiers, text, expected) in [
            (0x43, CONTROL, "\u{3}", android(31, META_CTRL)),
            (0x56, CONTROL | SHIFT, "\u{16}", android(50, META_CTRL | META_SHIFT)),
            (0x0100_0012, 0, "", android(21, 0)),
            (0x0100_0015, SHIFT, "", android(20, META_SHIFT)),
            (0x0100_0002, SHIFT, "", android(61, META_SHIFT)),
            (0x0100_0000, 0, "\u{1b}", android(4, 0)),
            (0x0100_0004, 0, "\r", android(66, 0)),
            (0x37, ALT, "7", android(14, META_ALT)),
            (0x41, SHIFT, "A", Some(Key::Text("A".into()))),
            (0xc9, 0, "é", Some(Key::Text("é".into()))),
            (0x0100_0020, SHIFT, "", None),
        ] {
            assert_eq!(map(key, modifiers, text), expected, "{key:#x} {modifiers:#x}");
        }
    }
}
