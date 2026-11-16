//! Working calendars: turning "working days from now" into dates (D20: the goal owner's calendar).
//!
//! Work counts from the **next working day** (decided 2026-11-16): something that takes 1 working
//! day and starts now finishes at the end of the next working day.

use std::collections::HashSet;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
/// Safety stop for date walks: about 77 years of working days.
const MAX_DAYS: u32 = 20_000;

#[derive(Clone, Debug)]
pub struct Calendar {
    /// Bit 0 = Monday … bit 6 = Sunday.
    workdays: u8,
    tz: TimeZone,
    days_off: HashSet<Date>,
}

/// Is `name` a time zone fluxo knows (IANA names such as `Europe/Lisbon`, or `UTC`)?
pub fn valid_time_zone(name: &str) -> bool {
    TimeZone::get(name).is_ok()
}

impl Default for Calendar {
    fn default() -> Self {
        Calendar { workdays: 0b001_1111, tz: TimeZone::UTC, days_off: HashSet::new() }
    }
}

impl Calendar {
    /// Unknown time zones fall back to UTC; an empty week falls back to Monday–Friday; unreadable
    /// days off are skipped.
    pub fn new(workdays_mask: i64, time_zone: &str, days_off: &[String]) -> Calendar {
        let workdays = match (workdays_mask & 0x7f) as u8 {
            0 => 0b001_1111,
            m => m,
        };
        Calendar {
            workdays,
            tz: TimeZone::get(time_zone).unwrap_or(TimeZone::UTC),
            days_off: days_off.iter().filter_map(|d| d.parse::<Date>().ok()).collect(),
        }
    }

    pub fn is_working(&self, d: Date) -> bool {
        let bit = d.weekday().to_monday_zero_offset() as u8;
        self.workdays & (1 << bit) != 0 && !self.days_off.contains(&d)
    }

    /// Today's date in this calendar's time zone.
    pub fn today(&self, unix_seconds: i64) -> Date {
        let ts = Timestamp::from_second(unix_seconds).unwrap_or(Timestamp::UNIX_EPOCH);
        ts.to_zoned_datetime(self.tz.clone()).date()
    }

    /// The first working day after today: where forecasts start.
    pub fn first_day(&self, unix_seconds: i64) -> Date {
        let mut d = self.today(unix_seconds);
        for _ in 0..MAX_DAYS {
            d = d.tomorrow().unwrap_or(d);
            if self.is_working(d) {
                break;
            }
        }
        d
    }

    /// The date on which `days` working days of work, counted from [`first_day`](Self::first_day),
    /// are done. Part days round up to the whole day; zero means the first day.
    pub fn finish_date(&self, unix_seconds: i64, days: f64) -> Date {
        let n = if days.is_finite() && days > 1e-9 { (days - 1e-9).ceil().min(MAX_DAYS as f64) as u32 } else { 1 };
        let mut d = self.first_day(unix_seconds);
        let mut counted = 1;
        let mut guard = 0;
        while counted < n && guard < MAX_DAYS * 2 {
            d = d.tomorrow().unwrap_or(d);
            if self.is_working(d) {
                counted += 1;
            }
            guard += 1;
        }
        d
    }
}

impl Calendar {
    /// Working days strictly after `a` up to and including `b` (negative if `b` is earlier).
    pub fn working_days_between(&self, a: Date, b: Date) -> i64 {
        let (lo, hi, sign) = if a <= b { (a, b, 1) } else { (b, a, -1) };
        let mut d = lo;
        let mut n = 0;
        for _ in 0..MAX_DAYS * 2 {
            if d >= hi {
                break;
            }
            d = d.tomorrow().unwrap_or(hi);
            if self.is_working(d) {
                n += 1;
            }
        }
        sign * n
    }
}

/// "Feb 12", or "Feb 12, 2027" when the year differs from `reference_year`.
pub fn label(d: Date, reference_year: i16) -> String {
    let m = MONTHS[(d.month() - 1) as usize];
    if d.year() == reference_year { format!("{m} {}", d.day()) } else { format!("{m} {}, {}", d.day(), d.year()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Monday 2026-11-16 12:00 UTC.
    const MON_NOON: i64 = 1_794_830_400;

    #[test]
    fn counts_from_the_next_working_day() {
        let c = Calendar::default();
        assert_eq!(c.today(MON_NOON).to_string(), "2026-11-16");
        assert_eq!(c.first_day(MON_NOON).to_string(), "2026-11-17");
        assert_eq!(c.finish_date(MON_NOON, 0.5).to_string(), "2026-11-17");
        assert_eq!(c.finish_date(MON_NOON, 1.0).to_string(), "2026-11-17");
        assert_eq!(c.finish_date(MON_NOON, 4.0).to_string(), "2026-11-20", "Tue…Fri");
        assert_eq!(c.finish_date(MON_NOON, 5.0).to_string(), "2026-11-23", "skips the weekend");
    }

    #[test]
    fn days_off_part_weeks_and_time_zones() {
        let c = Calendar::new(0b101, "UTC", &["2026-11-18".into(), "nonsense".into()]); // Mon, Wed
        assert_eq!(c.first_day(MON_NOON).to_string(), "2026-11-23", "Wed is a day off, so next Monday");
        assert_eq!(c.finish_date(MON_NOON, 2.0).to_string(), "2026-11-25");
        // Monday 23:30 in UTC is already Tuesday in Tokyo.
        let tokyo = Calendar::new(31, "Asia/Tokyo", &[]);
        assert_eq!(tokyo.today(MON_NOON + 11 * 3600 + 1800).to_string(), "2026-11-17");
        assert!(valid_time_zone("Europe/Lisbon"));
        assert!(!valid_time_zone("Mars/Olympus_Mons"));
        assert_eq!(Calendar::new(0, "Nowhere", &[]).first_day(MON_NOON).to_string(), "2026-11-17", "safe fallbacks");
    }

    #[test]
    fn working_days_between_dates() {
        let c = Calendar::default();
        let fri: Date = "2026-11-20".parse().unwrap();
        let tue: Date = "2026-11-24".parse().unwrap();
        assert_eq!(c.working_days_between(fri, tue), 2, "Mon and Tue");
        assert_eq!(c.working_days_between(tue, fri), -2);
        assert_eq!(c.working_days_between(fri, fri), 0);
    }

    #[test]
    fn labels() {
        let d: Date = "2027-02-12".parse().unwrap();
        assert_eq!(label(d, 2027), "Feb 12");
        assert_eq!(label(d, 2026), "Feb 12, 2027");
    }
}
