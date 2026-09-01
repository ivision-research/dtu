use std::fmt::Display;

/// Handles version matching for this crate
///
/// Note that `extra` is not considered for [PartialEq], use [Self::exact_match] instead
#[derive(Eq, Clone, Copy)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    pub extra: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/current_version.rs"));

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.major == other.major && self.minor == other.minor && self.patch == other.patch
    }
}

impl Version {
    /// Parse the major and minor version out of the given string, the returned
    /// Version has the patch set to 0 and no extra field
    pub fn from_major_minor(major_minor: &str) -> Option<Self> {
        let (major, minor) = major_minor.split_once('.')?;
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
            patch: 0,
            extra: None,
        })
    }

    /// Parse the full version, dropping any extra specifiers
    pub fn from_full(version: &str) -> Option<Self> {
        let (major, rem) = version.split_once('.')?;
        let (minor, rem) = rem.split_once('.')?;

        let patch = match rem.split_once('-') {
            Some((v, _)) => v,
            None => rem,
        };

        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
            patch: patch.parse().ok()?,
            extra: None,
        })
    }

    /// The only matching function that takes [Self::extra] into account
    pub fn exact_match(&self, other: Self) -> bool {
        self.major == other.major
            && self.minor == other.minor
            && self.patch == other.patch
            && match self.extra {
                None => other.extra.is_none(),
                Some(v) => other.extra.is_some_and(|it| it == v),
            }
    }

    pub fn major_matches(&self, other: Self) -> bool {
        self.major == other.major
    }

    pub fn major_minor_matches(&self, other: Self) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

impl Default for Version {
    fn default() -> Self {
        VERSION
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.extra {
            Some(s) => write!(f, "{}.{}.{}-{}", self.major, self.minor, self.patch, s),
            None => write!(f, "{}.{}.{}", self.major, self.minor, self.patch),
        }
    }
}
