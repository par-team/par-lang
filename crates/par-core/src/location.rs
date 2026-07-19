use arcstr::ArcStr;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Point {
    // 0-based
    pub offset: u32,
    // 0-based
    pub row: u32,
    /// Zero-based UTF-16 code units.
    pub column: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Span {
    None,
    At {
        start: Point,
        end: Point,
        file: FileName,
    },
}

impl Default for Span {
    fn default() -> Self {
        Self::None
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

pub trait Spanning {
    fn span(&self) -> Span;
}

impl Span {
    pub fn len(&self) -> u32 {
        match self {
            Self::None => 0,
            Self::At { start, end, .. } => end.offset - start.offset,
        }
    }

    pub fn points(&self) -> Option<(Point, Point)> {
        match self {
            Self::None => None,
            Self::At { start, end, .. } => Some((*start, *end)),
        }
    }

    pub fn file(&self) -> Option<FileName> {
        match self {
            Self::None => None,
            Self::At { file, .. } => Some(file.clone()),
        }
    }

    pub fn start(&self) -> Option<Point> {
        self.points().map(|(s, _)| s)
    }

    pub fn end(&self) -> Option<Point> {
        self.points().map(|(_, e)| e)
    }

    pub fn only_start(&self) -> Self {
        match self.clone() {
            Self::None => Self::None,
            Self::At { start, file, .. } => Self::At {
                start,
                end: start,
                file,
            },
        }
    }

    pub fn only_end(&self) -> Self {
        match self.clone() {
            Self::None => Self::None,
            Self::At { end, file, .. } => Self::At {
                start: end,
                end,
                file,
            },
        }
    }

    pub fn join(&self, other: Self) -> Self {
        match (self.clone(), other) {
            (Self::None, span) | (span, Self::None) => span,
            (
                Self::At {
                    start: start1,
                    end: end1,
                    file: file1,
                },
                Self::At {
                    start: start2,
                    end: end2,
                    file: file2,
                },
            ) => {
                assert_eq!(file1, file2, "can't join spans from different files");
                Self::At {
                    start: if start1.offset < start2.offset {
                        start1
                    } else {
                        start2
                    },
                    end: if end1.offset > end2.offset {
                        end1
                    } else {
                        end2
                    },
                    file: file1,
                }
            }
        }
    }
}

impl Display for Span {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "<unknown location>"),
            Self::At { start, file, .. } => {
                write!(f, "{}:{}:{}", file, start.row + 1, start.column + 1)
            }
        }
    }
}

impl Point {
    pub fn point_span(&self, file: FileName) -> Span {
        Span::At {
            start: *self,
            end: *self,
            file,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileName(pub ArcStr);

impl Display for FileName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl From<&str> for FileName {
    fn from(path: &str) -> Self {
        FileName(path.into())
    }
}

impl From<String> for FileName {
    fn from(path: String) -> Self {
        FileName(path.into())
    }
}

impl From<&Path> for FileName {
    fn from(path: &Path) -> Self {
        (&*path.to_string_lossy()).into()
    }
}

impl From<PathBuf> for FileName {
    fn from(path: PathBuf) -> Self {
        path.as_path().into()
    }
}

impl From<&FileName> for FileName {
    fn from(file: &FileName) -> Self {
        file.clone()
    }
}
