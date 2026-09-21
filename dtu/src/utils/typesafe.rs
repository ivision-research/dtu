#[allow(unused)]
macro_rules! typesafe_num {
    ($vis:vis $name:ident, $base:ty) => {
        typesafe_num!($vis $name, $base, "");
    };

    ($vis:vis $name:ident, $base:ty, $doc:literal) => {
        #[derive(Clone, Copy, Eq, PartialEq, Hash, Debug, Ord, PartialOrd, Default)]
        #[doc = $doc]
        $vis struct $name($base);

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::result::Result<(), ::std::fmt::Error> {
                self.0.fmt(f)
            }
        }

        impl $name {
            $vis const fn new(val: $base) -> Self {
                Self(val)
            }
        }

        impl ::std::convert::From<$base> for $name {
            fn from(val: $base) -> Self {
                Self(val)
            }
        }

        impl ::std::convert::From<$name> for $base {
            fn from(val: $name) -> Self {
                val.0
            }
        }

        impl ::std::ops::Add for $name {
            type Output = Self;
            fn add(self, other: Self) -> Self::Output {
                Self(self.0.add(other.0))
            }
        }

        impl ::std::ops::Sub for $name {
            type Output = Self;
            fn sub(self, other: Self) -> Self::Output {
                Self(self.0.sub(other.0))
            }
        }

        impl ::std::ops::BitAnd for $name {
            type Output = Self;
            fn bitand(self, other: Self) -> Self::Output {
                Self(self.0.bitand(other.0))
            }
        }

        impl ::std::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, other: Self) -> Self::Output {
                Self(self.0.bitor(other.0))
            }
        }

        impl ::std::ops::BitXor for $name {
            type Output = Self;
            fn bitxor(self, other: Self) -> Self::Output {
                Self(self.0.bitxor(other.0))
            }
        }

        impl ::std::ops::Not for $name {
            type Output = Self;
            fn not(self) -> Self::Output {
                Self(self.0.not())
            }
        }

        impl ::std::ops::Shr for $name {
            type Output = Self;
            fn shr(self, rhs: Self) -> Self::Output {
                Self(self.0.shr(rhs.0))
            }
        }

        impl ::std::ops::Shl for $name {
            type Output = Self;
            fn shl(self, rhs: Self) -> Self::Output {
                Self(self.0.shl(rhs.0))
            }
        }
    };
}

#[allow(unused)]
pub(crate) use typesafe_num;
