//! `claude --version` parsing (FR-PRM-33): the first `\d+\.\d+\.\d+` in the output.

use std::fmt;

/// A `major.minor.patch` version, ordered numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The minimum version (R88): the owner's probe verified every safety gate on 2.1.319, and
/// older versions were never checked. (`auth status` reports `configDirectory` from 2.1.268,
/// `--safe-mode` exists from 2.1.169.) The probe
/// fixes the real minimum, which is never lower.
pub const MIN_VERSION_FLOOR: Version = Version::new(2, 1, 319);

/// The first `\d+\.\d+\.\d+` in `text`, or `None`.
pub fn parse_version(text: &str) -> Option<Version> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            if let Some(v) = parse_at(&bytes[i..]) {
                return Some(v);
            }
            // Skip this digit run and try the next start.
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    None
}

fn parse_at(b: &[u8]) -> Option<Version> {
    let mut parts = [0u32; 3];
    let mut pos = 0;
    for (n, part) in parts.iter_mut().enumerate() {
        if n > 0 {
            if b.get(pos) != Some(&b'.') {
                return None;
            }
            pos += 1;
        }
        let start = pos;
        while b.get(pos).is_some_and(u8::is_ascii_digit) {
            pos += 1;
        }
        if pos == start {
            return None;
        }
        *part = std::str::from_utf8(&b[start..pos]).ok()?.parse().ok()?;
    }
    Some(Version::new(parts[0], parts[1], parts[2]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_33_version_parses_the_first_triple() {
        assert_eq!(
            parse_version("2.1.319 (Claude Code)"),
            Some(Version::new(2, 1, 319))
        );
        assert_eq!(
            parse_version("claude v10.0.2-beta"),
            Some(Version::new(10, 0, 2))
        );
        assert_eq!(
            parse_version("build 7, version 2.1.268."),
            Some(Version::new(2, 1, 268))
        );
        assert_eq!(parse_version("1.2 then 3.4.5"), Some(Version::new(3, 4, 5)));
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version("2.1"), None);
    }

    #[test]
    fn fr_prm_33_versions_order_numerically() {
        assert!(Version::new(2, 1, 319) >= MIN_VERSION_FLOOR);
        assert!(Version::new(2, 1, 318) < MIN_VERSION_FLOOR);
        assert!(Version::new(2, 1, 99) < MIN_VERSION_FLOOR);
        assert!(Version::new(2, 10, 0) > Version::new(2, 9, 999));
        assert_eq!(MIN_VERSION_FLOOR.to_string(), "2.1.319");
    }
}
