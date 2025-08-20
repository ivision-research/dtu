use std::fmt::Display;

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct Version {
    pub major: usize,
    pub minor: usize,
    pub patch: usize,
}

include!(concat!(env!("OUT_DIR"), "/current_version.rs"));

impl Version {
    /// Parse the major and minor version out of the given string, the returned
    /// Version has the patch set to 0
    pub fn from_major_minor(major_minor: &str) -> Option<Self> {
        let (major, minor) = major_minor.split_once('.')?;
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
            patch: 0,
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
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
