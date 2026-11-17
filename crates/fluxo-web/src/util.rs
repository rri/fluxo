//! Small formatting helpers for pages.

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Civil date (UTC) from Unix seconds. Howard Hinnant's days-from-civil, inverted.
pub fn civil(unix: i64) -> (i64, u32, u32) {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "Oct 2, 2026"
pub fn date(unix: i64) -> String {
    let (y, m, d) = civil(unix);
    format!("{} {d}, {y}", MONTHS[(m - 1) as usize])
}

/// "Active now", "12 minutes ago", "2 days ago"…
pub fn ago(unix: i64, now: i64) -> String {
    let s = (now - unix).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match s {
        0..120 => "Active now".into(),
        120..7_200 => plural(s / 60, "minute"),
        7_200..172_800 => plural(s / 3_600, "hour"),
        _ => plural(s / 86_400, "day"),
    }
}

/// Days since 1970-01-01 for a real YYYY-MM-DD date (`None` for "2026-02-30" and the like).
pub fn day_number(s: &str) -> Option<i64> {
    if !is_iso_date(s) {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (s[0..4].parse().ok()?, s[5..7].parse().ok()?, s[8..10].parse().ok()?);
    // Howard Hinnant's days-from-civil.
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let n = era * 146_097 + doe - 719_468;
    (iso(n) == s).then_some(n)
}

/// "2026-12-24" for a day number.
pub fn iso(day: i64) -> String {
    let (y, m, d) = civil(day * 86_400);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Is `s` a plausible YYYY-MM-DD date?
pub fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| s[r].parse::<u32>().ok();
    matches!((num(0..4), num(5..7), num(8..10)), (Some(1900..=2999), Some(1..=12), Some(1..=31)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date(0), "Jan 1, 1970");
        assert_eq!(date(1_791_331_200), "Oct 7, 2026");
        assert_eq!(civil(951_782_400), (2000, 2, 29));
        assert_eq!(ago(100, 130), "Active now");
        assert_eq!(ago(0, 3_600), "60 minutes ago");
        assert_eq!(ago(0, 3 * 86_400), "3 days ago");
        assert!(is_iso_date("2026-12-24"));
        assert!(!is_iso_date("2026-13-01"));
        assert!(!is_iso_date("24/12/2026"));
        assert_eq!(day_number("1970-01-02"), Some(1));
        assert_eq!(day_number("2000-02-29").map(iso).as_deref(), Some("2000-02-29"));
        assert_eq!(day_number("2026-02-30"), None);
    }
}
