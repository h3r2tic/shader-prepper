pub type BoxedIncludeProviderError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, thiserror::Error)]
pub enum PrepperError {
    /// Any error reported by the user-supplied `IncludeProvider`
    #[error("include provider error: \"{cause:?}\" when trying to include {file:?}")]
    IncludeProviderError {
        file: String,
        cause: BoxedIncludeProviderError,
    },

    /// Recursively included file, along with information about where it was encountered
    #[error("file {file:?} is recursively included; triggered in {from:?} ({from_line:?})")]
    RecursiveInclude {
        /// File which was included recursively
        file: String,

        /// File which included the recursively included one
        from: String,

        /// Line in the `from` file on which the include happened
        from_line: usize,
    },

    /// Error parsing an include directive
    #[error("parse error: {file:?} ({line:?})")]
    ParseError { file: String, line: usize },
}
