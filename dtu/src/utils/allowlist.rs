use std::borrow::{Borrow, ToOwned};
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::hash::Hash;
use std::io::{self, Read};
use std::path::Path;

use super::NewlineReader;

/// A simple allowlist based on a HashSet
pub struct Allowlist<T: Eq + Hash> {
    values: HashSet<T>,
}

impl<T: Eq + Hash + Clone> Clone for Allowlist<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
        }
    }
}

impl<T: Eq + Hash + Debug> Debug for Allowlist<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.values.fmt(f)
    }
}

impl<T: Eq + Hash> Extend<T> for Allowlist<T> {
    #[inline]
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.values.extend(iter)
    }
}

pub type OptAllowlist<T> = Option<Allowlist<T>>;

/// Convenience wrapper for optional allowlists
pub fn opt_allows<T, Q>(opt: &OptAllowlist<T>, val: &Q) -> bool
where
    T: Eq + Hash + Borrow<Q>,
    Q: Eq + Hash + ?Sized,
{
    opt.as_ref().map_or(true, |a| a.allows(val))
}

impl<T: Eq + Hash> Allowlist<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            values: HashSet::new(),
        }
    }

    #[inline]
    pub fn allows<Q: ?Sized>(&self, val: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.values.contains(val)
    }

    #[inline]
    pub fn push(&mut self, val: T) {
        self.values.insert(val);
    }

    #[inline]
    pub fn remove<Q: ?Sized>(&mut self, val: &Q)
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.values.remove(val);
    }

    /// Read the allowlist from a path that is a newline separated
    /// list of entries. This is just a basic wrapper around [from_nl_reader].
    pub fn from_path<P: AsRef<Path> + ?Sized, F: Fn(&str) -> T>(
        path: &P,
        transform: F,
    ) -> io::Result<Self> {
        let nl = NewlineReader::try_from(path.as_ref())?;
        Self::from_nl_reader(nl, transform)
    }

    /// Read the allowlist from a [NewlineReader]
    pub fn from_nl_reader<R: Read, F: Fn(&str) -> T>(
        reader: NewlineReader<R>,
        transform: F,
    ) -> io::Result<Self> {
        let mut values = HashSet::new();
        for l in reader {
            values.insert(transform(l?.as_str()));
        }
        Ok(Self { values })
    }
}

impl<R: Read> TryFrom<NewlineReader<R>> for Allowlist<String> {
    type Error = io::Error;

    fn try_from(reader: NewlineReader<R>) -> Result<Self, Self::Error> {
        let mut values = HashSet::new();
        for l in reader {
            values.insert(l?);
        }
        Ok(Self { values })
    }
}

impl<'a, T, Q, I> From<I> for Allowlist<T>
where
    T: Hash + Eq,
    Q: ToOwned<Owned = T> + 'a,
    I: IntoIterator<Item = &'a Q>,
{
    fn from(it: I) -> Self {
        let mut values = HashSet::new();
        for e in it {
            values.insert(e.to_owned());
        }
        Self { values }
    }
}

/// A denylist is just the inverse of an allowlist
pub struct Denylist<T: Eq + Hash>(Allowlist<T>);

impl<T: Eq + Hash + Clone> Clone for Denylist<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Eq + Hash + Debug> Debug for Denylist<T> {
    #[inline]
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T: Eq + Hash> Extend<T> for Denylist<T> {
    #[inline]
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.0.extend(iter)
    }
}

pub type OptDenylist<T> = Option<Denylist<T>>;

/// Convenience wrapper for optional denylists
pub fn opt_deny<T, Q>(opt: &OptDenylist<T>, val: &Q) -> bool
where
    T: Eq + Hash + Borrow<Q>,
    Q: Eq + Hash + ?Sized,
{
    opt.as_ref().map_or(false, |a| a.denies(val))
}

impl<T: Eq + Hash> Denylist<T> {
    #[inline]
    pub fn new() -> Self {
        Self(Allowlist::new())
    }

    #[inline]
    pub fn denies<Q: ?Sized>(&self, val: &Q) -> bool
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.0.values.contains(val)
    }

    #[inline]
    pub fn push(&mut self, val: T) {
        self.0.push(val)
    }

    #[inline]
    pub fn remove<Q: ?Sized>(&mut self, val: &Q)
    where
        T: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.0.remove(val);
    }

    /// Read the denylist from a path that is a newline separated
    /// list of entries. This is just a basic wrapper around [from_nl_reader].
    pub fn from_path<P: AsRef<Path> + ?Sized, F: Fn(&str) -> T>(
        path: &P,
        transform: F,
    ) -> io::Result<Self> {
        Ok(Self(Allowlist::from_path(path, transform)?))
    }

    /// Read the denylist from a [NewlineReader]
    pub fn from_nl_reader<R: Read, F: Fn(&str) -> T>(
        reader: NewlineReader<R>,
        transform: F,
    ) -> io::Result<Self> {
        Ok(Self(Allowlist::from_nl_reader(reader, transform)?))
    }
}

impl<R: Read> TryFrom<NewlineReader<R>> for Denylist<String> {
    type Error = <Allowlist<String> as TryFrom<NewlineReader<R>>>::Error;

    fn try_from(reader: NewlineReader<R>) -> Result<Self, Self::Error> {
        Ok(Self(Allowlist::try_from(reader)?))
    }
}

impl<'a, T, Q, I> From<I> for Denylist<T>
where
    T: Hash + Eq,
    Q: ToOwned<Owned = T> + 'a,
    I: IntoIterator<Item = &'a Q>,
{
    fn from(it: I) -> Self {
        Self(Allowlist::from(it))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_allowlist() {
        let lst = vec![1, 2];
        let allowlist = Allowlist::from(&lst);
        assert_eq!(allowlist.allows(&1), true);
        assert_eq!(allowlist.allows(&5), false);
        let lst = &["test", "list"];
        let allowlist = Allowlist::from(lst);
        assert_eq!(allowlist.allows("test"), true);
        assert_eq!(allowlist.allows("tes"), false);
        assert_eq!(allowlist.allows("nope"), false);
    }

    #[test]
    fn test_denylist() {
        let lst = vec![1, 2];
        let denylist = Denylist::from(&lst);
        assert_eq!(denylist.denies(&1), true);
        assert_eq!(denylist.denies(&5), false);
        let lst = &["test", "list"];
        let denylist = Denylist::from(lst);
        assert_eq!(denylist.denies("test"), true);
        assert_eq!(denylist.denies("tes"), false);
        assert_eq!(denylist.denies("nope"), false);
    }

    #[test]
    fn test_allowlist_from_nl() {
        let content = r#"#comment
content1
content2
#comment
     # comment
not# comment"#;

        let mut bytes = content.as_bytes();
        let reader = NewlineReader::new(&mut bytes);

        let al = Allowlist::try_from(reader).expect("should be ok");

        assert_eq!(al.allows("content1"), true);
        assert_eq!(al.allows("#comment"), false);
        assert_eq!(al.allows("not# comment"), true);
    }
}
