/// Chunk of source code along with information pointing back at the origin
#[derive(PartialEq, Eq, Debug)]
pub struct SourceChunk {
    /// Source text
    pub source: String,

    /// File the code came from
    pub file: String,

    /// Line in the `file` at which this snippet starts
    pub line_offset: usize,
}

impl SourceChunk {
    pub fn from_file_source(file: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            file: file.into(),
            line_offset: 0,
        }
    }
}
