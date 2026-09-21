use std::{
    borrow::Borrow,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    rc::Rc,
};

/// A Cow-esque type where the owned variant is reference counted making cloning cheaper for large
/// types
pub enum RcCow<'a, B>
where
    B: ?Sized + 'a,
    B: ToOwned,
{
    Owned(Rc<<B as ToOwned>::Owned>),
    Borrowed(&'a B),
}

impl<'a, B> RcCow<'a, B>
where
    B: ?Sized + ToOwned,
{
    pub fn owned(it: <B as ToOwned>::Owned) -> Self {
        Self::Owned(Rc::new(it))
    }
}

impl<'a, B> Deref for RcCow<'a, B>
where
    B: ?Sized + ToOwned,
{
    type Target = B;
    fn deref(&self) -> &Self::Target {
        match *self {
            Self::Borrowed(borrowed) => borrowed,
            Self::Owned(ref owned) => owned.as_ref().borrow(),
        }
    }
}

impl<'a, B> Borrow<B> for RcCow<'a, B>
where
    B: ?Sized + ToOwned,
{
    fn borrow(&self) -> &B {
        &**self
    }
}

impl<'a, B> Clone for RcCow<'a, B>
where
    B: ?Sized + ToOwned,
{
    fn clone(&self) -> Self {
        match *self {
            Self::Borrowed(b) => Self::Borrowed(b),
            Self::Owned(ref o) => Self::Owned(Rc::clone(o)),
        }
    }
}

impl<'a, B> PartialEq for RcCow<'a, B>
where
    B: ?Sized + PartialEq + ToOwned,
{
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(&**self, &**other)
    }
}

impl<'a, B> Eq for RcCow<'a, B> where B: ?Sized + Eq + ToOwned {}

impl<'a, B> PartialOrd for RcCow<'a, B>
where
    B: ?Sized + PartialOrd + ToOwned,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        PartialOrd::partial_cmp(&**self, &**other)
    }
}

impl<'a, B> Ord for RcCow<'a, B>
where
    B: ?Sized + Ord + ToOwned,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(&**self, &**other)
    }
}

impl<B> fmt::Display for RcCow<'_, B>
where
    B: ?Sized + fmt::Display + ToOwned<Owned: fmt::Display>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Borrowed(ref b) => fmt::Display::fmt(b, f),
            Self::Owned(ref o) => fmt::Display::fmt(o, f),
        }
    }
}

impl<B> fmt::Debug for RcCow<'_, B>
where
    B: ?Sized + fmt::Debug + ToOwned<Owned: fmt::Debug>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Borrowed(ref b) => fmt::Debug::fmt(b, f),
            Self::Owned(ref o) => fmt::Debug::fmt(o, f),
        }
    }
}

impl<B> Hash for RcCow<'_, B>
where
    B: ?Sized + Hash + ToOwned,
{
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&**self, state)
    }
}

impl<B> AsRef<B> for RcCow<'_, B>
where
    B: ?Sized + ToOwned,
{
    fn as_ref(&self) -> &B {
        self
    }
}

/// A custom Cow-esque type for shared immutable strings
///
/// This type only differs from Cow<'a, str> by instead having an `Rc<str>` inside the Owned variant
/// instead of a `String`. It's not a huge deal, but this makes it a bit more clear that this is
/// immutable and I think allows for a slightly better memory layout
pub enum RcStr<'a> {
    Borrowed(&'a str),
    Owned(Rc<str>),
}

impl<'a> RcStr<'a> {
    /// Create a new RcStr::Owned from the provided reference
    ///
    /// Note RcStr::from should be used instead if borrowing the str!
    pub fn own_str(s: &str) -> Self {
        Self::Owned(s.into())
    }
}

impl<'a> From<&'a str> for RcStr<'a> {
    fn from(value: &'a str) -> Self {
        Self::Borrowed(value)
    }
}

impl<'a> From<String> for RcStr<'a> {
    fn from(value: String) -> Self {
        Self::Owned(value.into())
    }
}

impl<'a> Deref for RcStr<'a> {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        match *self {
            Self::Borrowed(borrowed) => borrowed,
            Self::Owned(ref owned) => owned.as_ref(),
        }
    }
}

impl<'a> Borrow<str> for RcStr<'a> {
    fn borrow(&self) -> &str {
        &**self
    }
}

impl<'a> Clone for RcStr<'a> {
    fn clone(&self) -> Self {
        match *self {
            Self::Borrowed(b) => Self::Borrowed(b),
            Self::Owned(ref o) => Self::Owned(Rc::clone(o)),
        }
    }
}

impl<'a> PartialEq for RcStr<'a> {
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(&**self, &**other)
    }
}

impl<'a> Eq for RcStr<'a> {}

impl<'a> PartialOrd for RcStr<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        PartialOrd::partial_cmp(&**self, &**other)
    }
}

impl<'a> Ord for RcStr<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(&**self, &**other)
    }
}

impl<'a> fmt::Display for RcStr<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Borrowed(ref b) => fmt::Display::fmt(b, f),
            Self::Owned(ref o) => fmt::Display::fmt(o, f),
        }
    }
}

impl<'a> fmt::Debug for RcStr<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Borrowed(ref b) => fmt::Debug::fmt(b, f),
            Self::Owned(ref o) => fmt::Debug::fmt(o, f),
        }
    }
}

impl<'a> Hash for RcStr<'a> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        Hash::hash(&**self, state)
    }
}

impl<'a> AsRef<str> for RcStr<'a> {
    fn as_ref(&self) -> &str {
        self
    }
}
