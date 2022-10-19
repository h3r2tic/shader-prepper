//! **shader-prepper** is a shader include parser and crawler. It is mostly aimed at GLSL
//! which doesn't provide include directive support out of the box.
//!
//! This crate does not implement a full C-like preprocessor, only `#include` scanning.
//! Other directives are instead copied into the expanded code, so they can be subsequently
//! handled by the shader compiler.
//!
//! The API supports user-driven include file providers, which enable custom
//! virtual file systems, include paths, and allow build systems to track dependencies.
//!
//! Source files are not concatenated together, but returned as a Vec of [`SourceChunk`].
//! If a single string is needed, a `join` over the source strings can be used.
//! Otherwise, the individual chunks can be passed to the graphics API, and source info
//! contained within `SourceChunk` can then remap the compiler's errors back to
//! the original code.
//!
//! # Example
//!
//! ```rust
//!
//! struct FileIncludeProvider;
//! impl shader_prepper::IncludeProvider for FileIncludeProvider {
//!     type IncludeContext = ();
//!
//!     fn resolve_path(
//!         &self,
//!         path: &str,
//!         _context: &Self::IncludeContext,
//!     ) -> Result<shader_prepper::ResolvedInclude<Self::IncludeContext>, shader_prepper::BoxedIncludeProviderError>
//!     {
//!         Ok(shader_prepper::ResolvedInclude {
//!             resolved_path: shader_prepper::ResolvedIncludePath(path.to_owned()),
//!             context: (),
//!         })
//!     }
//!
//!     fn get_include(
//!         &mut self,
//!         resolved: &shader_prepper::ResolvedIncludePath,
//!     ) -> Result<String, shader_prepper::BoxedIncludeProviderError> {
//!         Ok(std::fs::read_to_string(&resolved.0)?)
//!     }
//! }
//!
//! // ...
//!
//! let chunks = shader_prepper::process_file("myfile.glsl", &mut FileIncludeProvider, ());
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]

mod error;
mod include_provider;
mod scanner;
mod source_chunk;

#[cfg(test)]
mod tests;

#[cfg_attr(docsrs, doc(cfg(feature = "gl_compiler")))]
#[cfg(feature = "gl_compiler")]
pub mod gl_compiler;

use std::collections::HashSet;

use scanner::Scanner;
pub use {error::*, include_provider::*, source_chunk::*};

/// Process a single file, and then any code recursively referenced.
///
/// `include_provider` is used to read all of the files, including the one at `file_path`.
pub fn process_file<IncludeContext: Clone>(
    file_path: &str,
    include_provider: &mut dyn IncludeProvider<IncludeContext = IncludeContext>,
    include_context: IncludeContext,
) -> Result<Vec<SourceChunk<IncludeContext>>, BoxedIncludeProviderError> {
    let mut prior_includes = HashSet::new();
    let mut skip_includes = HashSet::new();

    let mut scanner = Scanner::new(
        "",
        String::new(),
        &mut prior_includes,
        &mut skip_includes,
        include_provider,
        include_context,
    );
    scanner.include_child(file_path, 1)?;
    Ok(scanner.into_chunks())
}
