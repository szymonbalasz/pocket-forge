//! UTC timestamps with millisecond precision, written as RFC 3339 (manifest-schema §1).

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Milliseconds since the Unix epoch, UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn from_system(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp(d.as_millis() as i64),
            Err(e) => Timestamp(-(e.duration().as_millis() as i64)),
        }
    }

    pub fn millis(self) -> i64 {
        self.0
    }

    pub fn plus(self, d: Duration) -> Self {
        Timestamp(self.0 + d.as_millis() as i64)
    }

    fn parts(self) -> (i64, u32, u32, u32, u32, u32, u32) {
        let ms = self.0.rem_euclid(1000) as u32;
        let secs = self.0.div_euclid(1000);
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400) as u32;
        let (y, m, d) = civil_from_days(days);
        (y, m, d, sod / 3600, (sod / 60) % 60, sod % 60, ms)
    }

    /// Compact UTC form for file names: `20261107-142501`.
    pub fn file_stamp(self) -> String {
        let (y, mo, d, h, mi, s, _) = self.parts();
        format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, mo, d, h, mi, s, ms) = self.parts();
        write!(f, "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
    }
}

/// Error parsing an RFC 3339 timestamp.
#[derive(Debug, thiserror::Error)]
#[error("not an RFC 3339 UTC timestamp: {0:?}")]
pub struct ParseTimestampError(String);

impl FromStr for Timestamp {
    type Err = ParseTimestampError;

    /// Parses `YYYY-MM-DDTHH:MM:SS[.fff]Z` (UTC only; offsets other than `Z` / `+00:00` fail).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseTimestampError(s.to_owned());
        let body = s
            .strip_suffix('Z')
            .or_else(|| s.strip_suffix("+00:00"))
            .ok_or_else(err)?;
        let (date, time) = body.split_once(['T', ' ']).ok_or_else(err)?;
        let num = |t: &str| t.parse::<i64>().map_err(|_| err());
        let mut d = date.split('-');
        let (y, mo, da) = (
            num(d.next().ok_or_else(err)?)?,
            num(d.next().ok_or_else(err)?)?,
            num(d.next().ok_or_else(err)?)?,
        );
        let (hms, frac) = time.split_once('.').unwrap_or((time, "0"));
        let mut t = hms.split(':');
        let (h, mi, sec) = (
            num(t.next().ok_or_else(err)?)?,
            num(t.next().ok_or_else(err)?)?,
            num(t.next().ok_or_else(err)?)?,
        );
        if !(1..=12).contains(&mo)
            || !(1..=31).contains(&da)
            || h > 23
            || mi > 59
            || sec > 60
            || frac.is_empty()
            || !frac.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(err());
        }
        let ms = num(&format!("{frac:0<3}")[..3])?;
        let days = days_from_civil(y, mo as u32, da as u32);
        Ok(Timestamp(
            ((days * 86_400 + h * 3600 + mi * 60 + sec) * 1000) + ms,
        ))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

// Howard Hinnant's civil-date algorithms (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_rfc3339_with_millis() {
        assert_eq!(Timestamp(0).to_string(), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            Timestamp(1_793_974_245_123).to_string(),
            "2026-11-06T14:10:45.123Z"
        );
        assert_eq!(Timestamp(1_793_974_245_123).file_stamp(), "20261106-141045");
    }

    #[test]
    fn round_trips_across_dates() {
        for ms in [
            0,
            951_782_400_000,
            1_709_164_800_999,
            4_102_444_800_000,
            -86_400_000,
        ] {
            let t = Timestamp(ms);
            assert_eq!(t.to_string().parse::<Timestamp>().unwrap(), t, "{t}");
        }
    }

    #[test]
    fn parses_variants_and_rejects_garbage() {
        assert_eq!(
            "2026-11-07T00:00:00Z".parse::<Timestamp>().unwrap(),
            "2026-11-07T00:00:00.000+00:00"
                .parse::<Timestamp>()
                .unwrap()
        );
        assert_eq!(
            "1970-01-01T00:00:00.5Z".parse::<Timestamp>().unwrap(),
            Timestamp(500)
        );
        for bad in [
            "",
            "2026-11-07",
            "2026-13-01T00:00:00Z",
            "2026-11-07T00:00:00+02:00",
            "x",
        ] {
            assert!(bad.parse::<Timestamp>().is_err(), "{bad}");
        }
    }
}
