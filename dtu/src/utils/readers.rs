use std::fs::File;
use std::io::{self, BufRead, BufReader, Lines, Read};
use std::path::{Path, PathBuf};

pub struct NewlineReader<R: Read> {
    comment_marker: Option<char>,
    lines: Lines<BufReader<R>>,
}

impl<R: Read> From<R> for NewlineReader<R> {
    fn from(value: R) -> Self {
        Self::new(value)
    }
}

macro_rules! try_from {
    ($src:ty) => {
        impl TryFrom<$src> for NewlineReader<File> {
            type Error = io::Error;

            fn try_from(path: $src) -> Result<Self, Self::Error> {
                let f = File::open(path)?;
                Ok(Self::new(f))
            }
        }
    };
}

try_from!(&Path);
try_from!(PathBuf);
try_from!(&PathBuf);

impl<R: Read> NewlineReader<R> {
    pub fn new(wrapped: R) -> Self {
        let lines = BufReader::new(wrapped).lines();
        Self {
            lines,
            comment_marker: Some('#'),
        }
    }

    pub fn set_comment_marker(mut self, marker: Option<char>) -> Self {
        self.comment_marker = marker;
        self
    }

    fn line_is_comment(&self, line: &str) -> bool {
        self.comment_marker
            .map_or(false, |c| line.trim().starts_with(c))
    }
}

impl<R: Read> Iterator for NewlineReader<R> {
    type Item = io::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.lines.next() {
                None => return None,
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(l)) => {
                    if self.line_is_comment(&l) {
                        continue;
                    }
                    return Some(Ok(l));
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::utils::NewlineReader;

    #[test]
    fn test_newline_reader() {
        let content = r#"#comment
content1
content2
#comment
     # comment
not# comment"#;

        let mut bytes = content.as_bytes();
        let reader = NewlineReader::new(&mut bytes);
        let lines = reader
            .map(|it| it.expect("no errors"))
            .collect::<Vec<String>>();
        assert_eq!(lines.as_slice(), &["content1", "content2", "not# comment"])
    }
}
