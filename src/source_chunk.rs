/// Chunk of source code along with information pointing back at the origin
#[derive(PartialEq, Eq, Debug)]
pub struct SourceChunk<IncludeContext> {
    /// Source text
    pub source: String,

    /// Context from the include provider
    pub context: IncludeContext,

    /// File the code came from; only the leaf of the path.
    /// For nested file information, use a custom `IncludeContext`.
    pub file: String,

    /// Line in the `file` at which this snippet starts
    pub line_offset: usize,
}
