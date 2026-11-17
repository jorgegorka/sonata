use chrono::{DateTime, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};

pub const TASK_NAME_MAX: usize = 40;
pub const EVERY_MIN: u32 = 1;
pub const EVERY_MAX: u32 = 365;
pub const DEFAULT_AT: &str = "09:00";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub every_days: u32,
    pub at: NaiveTime,
    pub starts_on: NaiveDate,
}

impl Schedule {
    pub fn slot<Tz: TimeZone>(&self, k: i64, tz: &Tz) -> DateTime<Utc> {
        let date = self.starts_on + Duration::days(k * i64::from(self.every_days));
        let naive = date.and_time(self.at);
        let local = tz
            .from_local_datetime(&naive)
            .earliest()
            .or_else(|| tz.from_local_datetime(&(naive + Duration::hours(1))).earliest());
        match local {
            Some(t) => t.with_timezone(&Utc),
            None => naive.and_utc(),
        }
    }

    pub fn first_slot_at_or_after<Tz: TimeZone>(&self, t: DateTime<Utc>, tz: &Tz) -> DateTime<Utc> {
        self.first_slot_where(t, tz, |slot| slot >= t)
    }

    pub fn first_slot_after<Tz: TimeZone>(&self, t: DateTime<Utc>, tz: &Tz) -> DateTime<Utc> {
        self.first_slot_where(t, tz, |slot| slot > t)
    }

    pub fn last_slot_at_or_before<Tz: TimeZone>(&self, t: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        let k = self.first_index_where(t, tz, |slot| slot > t);
        (k > 0).then(|| self.slot(k - 1, tz))
    }

    fn first_slot_where<Tz: TimeZone>(
        &self,
        t: DateTime<Utc>,
        tz: &Tz,
        ok: impl Fn(DateTime<Utc>) -> bool,
    ) -> DateTime<Utc> {
        self.slot(self.first_index_where(t, tz, ok), tz)
    }

    fn first_index_where<Tz: TimeZone>(&self, t: DateTime<Utc>, tz: &Tz, ok: impl Fn(DateTime<Utc>) -> bool) -> i64 {
        let days = (local_date(t, tz) - self.starts_on).num_days();
        let mut k = (days / i64::from(self.every_days.max(1))).max(0);
        while !ok(self.slot(k, tz)) {
            k += 1;
        }
        k
    }
}

pub fn parse_at(text: &str) -> Option<NaiveTime> {
    let (h, m) = text.split_once(':')?;
    let digits = |s: &str, max: usize| (1..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(h, 2) || m.len() != 2 || !digits(m, 2) {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

pub fn format_at(at: NaiveTime) -> String {
    format!("{:02}:{:02}", at.hour(), at.minute())
}

pub fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

pub fn default_start<Tz: TimeZone>(at: NaiveTime, now: DateTime<Utc>, tz: &Tz) -> NaiveDate {
    let today = local_date(now, tz);
    let first = Schedule { every_days: 1, at, starts_on: today }.slot(0, tz);
    if first > now {
        today
    } else {
        today + Duration::days(1)
    }
}

pub fn local_minute<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz) -> String {
    t.with_timezone(tz).naive_local().format("%Y-%m-%d %H:%M").to_string()
}

pub fn local_date<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz) -> NaiveDate {
    t.with_timezone(tz).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, Utc};

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    fn utc(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn slots_are_the_start_date_plus_k_periods_at_the_time() {
        let s =
            Schedule { every_days: 7, at: parse_at("09:00").unwrap(), starts_on: parse_date("2027-06-07").unwrap() };
        assert_eq!(s.slot(0, &tz()).to_rfc3339(), "2027-06-07T07:00:00+00:00");
        assert_eq!(s.slot(2, &tz()).to_rfc3339(), "2027-06-21T07:00:00+00:00");
    }

    #[test]
    fn the_first_slot_after_keeps_the_grid() {
        let s =
            Schedule { every_days: 7, at: parse_at("09:00").unwrap(), starts_on: parse_date("2027-06-07").unwrap() };
        let tuesday = utc("2027-06-15T10:00:00Z");
        assert_eq!(s.first_slot_after(tuesday, &tz()), s.slot(2, &tz()));
        assert_eq!(s.first_slot_after(s.slot(1, &tz()), &tz()), s.slot(2, &tz()));
        assert_eq!(s.first_slot_at_or_after(s.slot(1, &tz()), &tz()), s.slot(1, &tz()));
        assert_eq!(s.first_slot_at_or_after(utc("2027-01-01T00:00:00Z"), &tz()), s.slot(0, &tz()));
        assert_eq!(s.first_slot_after(utc("2028-06-07T08:00:00Z"), &tz()).to_rfc3339(), "2028-06-12T07:00:00+00:00");
    }

    #[test]
    fn the_last_slot_at_or_before_is_the_one_before_the_next() {
        let s =
            Schedule { every_days: 7, at: parse_at("09:00").unwrap(), starts_on: parse_date("2027-06-07").unwrap() };
        assert_eq!(s.last_slot_at_or_before(utc("2027-06-29T10:00:00Z"), &tz()), Some(s.slot(3, &tz())));
        assert_eq!(s.last_slot_at_or_before(s.slot(1, &tz()), &tz()), Some(s.slot(1, &tz())));
        assert_eq!(s.last_slot_at_or_before(utc("2027-06-07T06:59:00Z"), &tz()), None);
    }

    #[test]
    fn parse_at_takes_one_or_two_digit_hours_only() {
        assert_eq!(parse_at("9:00").map(format_at).as_deref(), Some("09:00"));
        assert_eq!(parse_at("23:59").map(format_at).as_deref(), Some("23:59"));
        for bad in ["24:00", "9", "09:60", "9am", "09:0", " 09:00", "", "123:00"] {
            assert_eq!(parse_at(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_default_start_is_today_while_the_time_is_ahead_else_tomorrow() {
        let at = parse_at("09:00").unwrap();
        assert_eq!(default_start(at, utc("2027-06-07T06:59:00Z"), &tz()), parse_date("2027-06-07").unwrap());
        assert_eq!(default_start(at, utc("2027-06-07T07:00:00Z"), &tz()), parse_date("2027-06-08").unwrap());
    }

    #[test]
    fn local_text_is_minutes_in_the_zone() {
        assert_eq!(local_minute(utc("2027-06-07T07:00:30Z"), &tz()), "2027-06-07 09:00");
    }
}
