//! Text the pages show for times, sizes, and values, in the local time zone.

use std::time::{SystemTime, UNIX_EPOCH};

use cxx_qt_lib::{QDate, QDateTime, QString, QTimeZone, QVariant, QVariantValue};

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

pub fn variant<T: QVariantValue>(value: &T) -> QVariant {
    QVariant::from(value)
}

pub fn text(value: &str) -> QVariant {
    variant(&QString::from(value))
}

pub fn file_url(path: &str) -> String {
    format!("file://{path}")
}

/// "now", "5 min", "3 h", "2 d".
pub fn age_text(at: u64) -> String {
    match now().saturating_sub(at) / 60 {
        0 => "now".to_owned(),
        minutes @ 1..60 => format!("{minutes} min"),
        minutes @ 60..1440 => format!("{} h", minutes / 60),
        minutes => format!("{} d", minutes / 1440),
    }
}

fn local(secs: u64) -> QDateTime {
    QDateTime::from_secs_since_epoch(i64::try_from(secs).unwrap_or(i64::MAX), &QTimeZone::system_time_zone())
}

fn date(secs: u64, format: &str) -> String {
    local(secs).date().format(&QString::from(format)).to_string()
}

/// Days from the date of `secs` to today: 0 today, 1 yesterday.
fn days_ago(secs: u64) -> i64 {
    local(secs).date().days_to(QDateTime::current_date_time().date())
}

fn same_year(secs: u64) -> bool {
    let year = |date: QDate| date.year();
    year(local(secs).date()) == year(QDateTime::current_date_time().date())
}

/// "14:02"
pub fn clock(secs: u64) -> String {
    let time = local(secs).time();
    format!("{:02}:{:02}", time.hour(), time.minute())
}

/// "Thu 29 Oct", or "Thu 29 Oct 2024" in another year.
pub fn day_detail(secs: u64) -> String {
    date(secs, if same_year(secs) { "ddd d MMM" } else { "ddd d MMM yyyy" })
}

/// "Today", "Yesterday", else as `day_detail`.
pub fn day_text(secs: u64) -> String {
    match days_ago(secs) {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => day_detail(secs),
    }
}

/// "Today", "Yesterday", "Wednesday" within the week, else "12 Oct".
pub fn day_name(secs: u64) -> String {
    match days_ago(secs) {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..7 => date(secs, "dddd"),
        _ => date(secs, if same_year(secs) { "d MMM" } else { "d MMM yyyy" }),
    }
}

/// For a list of conversations: "14:02" today, "Yesterday", "Mon" within the week, else "12 Oct".
pub fn short_when(secs: u64) -> String {
    match days_ago(secs) {
        0 => clock(secs),
        1 => "Yesterday".to_owned(),
        2..7 => date(secs, "ddd"),
        _ => date(secs, if same_year(secs) { "d MMM" } else { "d MMM yyyy" }),
    }
}

/// For a file: "Today 14:02", "Yesterday 09:15", "12 Oct", "12 Oct 2024".
pub fn modified(secs: u64) -> String {
    match days_ago(secs) {
        0 => format!("Today {}", clock(secs)),
        1 => format!("Yesterday {}", clock(secs)),
        _ => date(secs, if same_year(secs) { "d MMM" } else { "d MMM yyyy" }),
    }
}

/// "812 B", "12.4 MB", "1.2 GB".
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    #[expect(clippy::cast_precision_loss, reason = "a size shown to one decimal")]
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value < 10.0 { format!("{value:.1} {}", UNITS[unit]) } else { format!("{value:.0} {}", UNITS[unit]) }
}

/// The lower-case extension of a file name.
pub fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

/// image, video, audio, document, archive, or file.
pub fn file_kind(name: &str) -> &'static str {
    match extension(name).as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "bmp" | "dng" => "image",
        "mp4" | "mkv" | "webm" | "mov" | "3gp" | "avi" => "video",
        "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" | "aac" => "audio",
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" | "csv" => {
            "document"
        }
        "zip" | "tar" | "gz" | "7z" | "rar" | "xz" | "zst" | "apk" => "archive",
        _ => "file",
    }
}
