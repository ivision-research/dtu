use std::fmt::Display;

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
    pub extra: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/current_version.rs"));

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
