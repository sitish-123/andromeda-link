//! Lets a decoder show each H.264 picture as soon as it is decoded. An SPS with picture order type 2 (output order is
//! decode order) whose VUI sets `max_dec_frame_buffering` to the reference count leaves the decoder's picture buffer
//! full after every picture; `GStreamer` decoders then output a picture only when the next one arrives. One more
//! slot lets them output at once.

/// Rewrites each SPS in an Annex B access unit so the picture buffer has a free slot. Other units stay as they are.
pub fn rewrite(unit: &mut Vec<u8>) {
    let mut at = 0;
    while let Some((start, end)) = next_nal(unit, at) {
        at = end;
        if unit[start] & 0x1f != 7 {
            continue;
        }
        let Some(sps) = free_slot(&unescape(&unit[start..end])) else { continue };
        let sps = escape(&sps);
        at = start + sps.len();
        unit.splice(start..end, sps);
    }
}

/// The next NAL unit at or after `from`, without its start code: its first header byte and its end.
fn next_nal(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let start = find_start(bytes, from).filter(|&start| start < bytes.len())?;
    let end = find_start(bytes, start).map_or(bytes.len(), |next| {
        let mut end = next - 3;
        while end > start && bytes[end - 1] == 0 {
            end -= 1;
        }
        end
    });
    Some((start, end))
}

/// The byte after the next `00 00 01` at or after `from`.
fn find_start(bytes: &[u8], from: usize) -> Option<usize> {
    bytes.get(from..)?.windows(3).position(|w| w == [0, 0, 1]).map(|at| from + at + 3)
}

fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0;
    for &byte in nal {
        if byte == 3 && zeros >= 2 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

fn escape(rbsp: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rbsp.len() + 4);
    let mut zeros = 0;
    for &byte in rbsp {
        if byte <= 3 && zeros >= 2 {
            out.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        out.push(byte);
    }
    out
}

/// Where an SPS's fields sit, in bits from the start of its header byte.
struct Layout {
    refs: u32,
    /// `vui_parameters_present_flag`.
    vui: usize,
    /// `bitstream_restriction_flag`, when the SPS has a VUI.
    restriction: Option<usize>,
}

/// The layout of an SPS with picture order type 2, or `None` for any other.
fn layout(sps: &[u8]) -> Option<Layout> {
    let mut r = Reader { bytes: sps, bit: 8 };
    let profile = r.bits(8)?;
    r.bits(16)?;
    r.ue()?;
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma = r.ue()?;
        if chroma == 3 {
            r.bits(1)?;
        }
        r.ue()?;
        r.ue()?;
        r.bits(1)?;
        if r.bits(1)? == 1 {
            for list in 0..if chroma == 3 { 12 } else { 8 } {
                if r.bits(1)? == 1 {
                    r.scaling_list(if list < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    r.ue()?;
    if r.ue()? != 2 {
        return None;
    }
    let refs = r.ue()?;
    r.bits(1)?;
    r.ue()?;
    r.ue()?;
    if r.bits(1)? == 0 {
        r.bits(1)?;
    }
    r.bits(1)?;
    if r.bits(1)? == 1 {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    let vui = r.bit;
    if r.bits(1)? == 0 {
        return Some(Layout { refs, vui, restriction: None });
    }
    r.vui_to_restriction()?;
    Some(Layout { refs, vui, restriction: Some(r.bit) })
}

/// `motion_vectors_over_pic_boundaries_flag`, the four limits after it, `max_num_reorder_frames` and
/// `max_dec_frame_buffering`, read after a set `bitstream_restriction_flag`.
fn restriction(sps: &[u8], flag: usize) -> Option<(u32, [u32; 4], u32, u32)> {
    let mut r = Reader { bytes: sps, bit: flag };
    if r.bits(1)? == 0 {
        return None;
    }
    let motion = r.bits(1)?;
    let limits = [r.ue()?, r.ue()?, r.ue()?, r.ue()?];
    Some((motion, limits, r.ue()?, r.ue()?))
}

/// The SPS (header byte included, no emulation bytes) with one free picture slot, or `None` to keep it.
fn free_slot(sps: &[u8]) -> Option<Vec<u8>> {
    let layout = layout(sps)?;
    let mut w = Writer::default();
    let (motion, limits) = match layout.restriction {
        None => {
            w.copy(sps, layout.vui);
            w.put(1, 1);
            w.put(0, 8);
            (1, [2, 1, 16, 16])
        }
        Some(flag) => {
            w.copy(sps, flag);
            match restriction(sps, flag) {
                Some((_, _, _, buffering)) if buffering > layout.refs => return None,
                Some((motion, limits, _, _)) => (motion, limits),
                None => (1, [2, 1, 16, 16]),
            }
        }
    };
    w.put(1, 1);
    w.put(motion, 1);
    for limit in limits {
        w.ue(limit);
    }
    w.ue(0);
    w.ue(layout.refs + 1);
    w.trailing();
    Some(w.bytes)
}

struct Reader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl Reader<'_> {
    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            let byte = self.bytes.get(self.bit / 8)?;
            value = value << 1 | u32::from((byte >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Some(value)
    }

    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1 << zeros) - 1 + self.bits(zeros)?)
    }

    fn se(&mut self) -> Option<i64> {
        let code = i64::from(self.ue()?);
        Some(if code % 2 == 1 { (code + 1) / 2 } else { -code / 2 })
    }

    fn scaling_list(&mut self, size: usize) -> Option<()> {
        let (mut last, mut next) = (8, 8);
        for _ in 0..size {
            if next != 0 {
                next = (last + self.se()? + 256) % 256;
            }
            if next != 0 {
                last = next;
            }
        }
        Some(())
    }

    fn hrd(&mut self) -> Option<()> {
        let count = self.ue()? + 1;
        self.bits(8)?;
        for _ in 0..count {
            self.ue()?;
            self.ue()?;
            self.bits(1)?;
        }
        self.bits(20).map(drop)
    }

    /// Reads the VUI up to `bitstream_restriction_flag`.
    fn vui_to_restriction(&mut self) -> Option<()> {
        if self.bits(1)? == 1 && self.bits(8)? == 255 {
            self.bits(32)?;
        }
        if self.bits(1)? == 1 {
            self.bits(1)?;
        }
        if self.bits(1)? == 1 {
            self.bits(4)?;
            if self.bits(1)? == 1 {
                self.bits(24)?;
            }
        }
        if self.bits(1)? == 1 {
            self.ue()?;
            self.ue()?;
        }
        if self.bits(1)? == 1 {
            self.bits(32)?;
            self.bits(32)?;
            self.bits(1)?;
        }
        let nal = self.bits(1)? == 1;
        if nal {
            self.hrd()?;
        }
        let vcl = self.bits(1)? == 1;
        if vcl {
            self.hrd()?;
        }
        if nal || vcl {
            self.bits(1)?;
        }
        self.bits(1).map(drop)
    }
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
    bit: usize,
}

impl Writer {
    fn put(&mut self, value: u32, count: u32) {
        for shift in (0..count).rev() {
            if self.bit.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if let Some(last) = self.bytes.last_mut() {
                *last |= u8::try_from((value >> shift) & 1).unwrap_or(0) << (7 - self.bit % 8);
            }
            self.bit += 1;
        }
    }

    fn ue(&mut self, value: u32) {
        let code = u64::from(value) + 1;
        let length = 64 - code.leading_zeros();
        self.put(0, length - 1);
        for shift in (0..length).rev() {
            self.put(u32::from((code >> shift) & 1 == 1), 1);
        }
    }

    /// The first `count` bits of `bytes`.
    fn copy(&mut self, bytes: &[u8], count: usize) {
        let mut r = Reader { bytes, bit: 0 };
        for _ in 0..count {
            self.put(r.bits(1).unwrap_or(0), 1);
        }
    }

    fn trailing(&mut self) {
        self.put(1, 1);
        while !self.bit.is_multiple_of(8) {
            self.put(0, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The phone encoder (a `OnePlus` 12)'s first unit at 1440x3168 (SPS, PPS, start of the IDR slice).
    const PHONE: &str = "000000016742c034da00c602d69a8283028368509a800000000168ce06f20000000165b840f72468a000214b26e4";

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap_or(0)).collect()
    }

    /// `max_num_reorder_frames` and `max_dec_frame_buffering` of the unit's SPS.
    fn reorder_and_buffering(unit: &[u8]) -> Option<(u32, u32)> {
        let (start, end) = next_nal(unit, 0)?;
        let sps = unescape(&unit[start..end]);
        let (_, _, reorder, buffering) = restriction(&sps, layout(&sps)?.restriction?)?;
        Some((reorder, buffering))
    }

    #[test]
    fn emulation_bytes_round_trip() {
        let rbsp = [0x67, 0, 0, 3, 0, 0, 0, 0, 1, 0, 0, 2];
        assert_eq!(escape(&rbsp), [0x67, 0, 0, 3, 3, 0, 0, 3, 0, 0, 3, 1, 0, 0, 3, 2]);
        assert_eq!(unescape(&escape(&rbsp)), rbsp);
    }

    #[test]
    fn a_unit_ending_in_a_start_code_stays() {
        let mut unit = vec![0, 0, 1, 0x41, 9, 0, 0, 1];
        rewrite(&mut unit);
        assert_eq!(unit, [0, 0, 1, 0x41, 9, 0, 0, 1]);
    }

    #[test]
    fn the_phone_sps_gets_a_free_slot_and_the_rest_stays() {
        let original = hex(PHONE);
        assert_eq!(reorder_and_buffering(&original), Some((0, 1)));
        let mut unit = original.clone();
        rewrite(&mut unit);
        assert_eq!(reorder_and_buffering(&unit), Some((0, 2)));
        let tail = |bytes: &[u8]| bytes[find_start(bytes, 4).unwrap_or(0) - 4..].to_vec();
        assert_eq!(tail(&unit), tail(&original));
        let again = unit.clone();
        rewrite(&mut unit);
        assert_eq!(unit, again);
    }
}
