//! Compiling OpenGL shaders and reporting errors from the process is somewhat involved.
//! OpenGL doesn't support include files, and its error logs therefore do not reference files.
//!
//! Instead, OpenGL operates at the level of source strings, and reports indices into an array
//! of those strings when reporting errors. On top of that, the log output format is vendor-specific.
//!
//! This module provides the `compile_shader` function, which helps simplify the process:
//! for each `SourceChunk`, it creates a `String` decorated with a `#line` pragma,
//! calls the user-provided compiler callback, and then parses its output, mapping
//! integral source locations to paths used by the `IncludeProvider`.
//!
//! An example implementation using `compile_shader` and the `gl_generator` crate:
//!
//! ```rust
//! fn make_shader<'chunk>(
//!     gl: &gl::Gl,
//!     shader_type: GLenum,
//!     sources: impl Iterator<Item = &'chunk shader_prepper::SourceChunk>,
//! ) -> anyhow::Result<u32> {
//!     unsafe {
//!         let compiled_shader = compile_shader(sources, |sources| {
//!             let handle = gl.CreateShader(shader_type);
//!
//!             let (source_lengths, source_ptrs): (Vec<GLint>, Vec<*const GLchar>) = sources
//!                 .iter()
//!                 .map(|s| (s.len() as GLint, s.as_ptr() as *const GLchar))
//!                 .unzip();
//!
//!             gl.ShaderSource(
//!                 handle,
//!                 source_ptrs.len() as i32,
//!                 source_ptrs.as_ptr(),
//!                 source_lengths.as_ptr(),
//!             );
//!             gl.CompileShader(handle);
//!
//!             let mut shader_ok: gl::types::GLint = 1;
//!             gl.GetShaderiv(handle, gl::COMPILE_STATUS, &mut shader_ok);
//!
//!             if shader_ok != 1 {
//!                 let mut log_len: gl::types::GLint = 0;
//!                 gl.GetShaderiv(handle, gl::INFO_LOG_LENGTH, &mut log_len);
//!
//!                 let log_str = CString::from_vec_unchecked(vec![b'\0'; (log_len + 1) as usize]);
//!                 gl.GetShaderInfoLog(
//!                     handle,
//!                     log_len,
//!                     std::ptr::null_mut(),
//!                     log_str.as_ptr() as *mut gl::types::GLchar,
//!                 );
//!
//!                 gl.DeleteShader(handle);
//!
//!                 ShaderCompilerOutput {
//!                     artifact: None,
//!                     log: Some(log_str.to_string_lossy().into_owned()),
//!                 }
//!             } else {
//!                 ShaderCompilerOutput {
//!                     artifact: Some(handle),
//!                     log: None,
//!                 }
//!             }
//!         });
//!
//!         if let Some(shader) = compiled_shader.artifact {
//!             if let Some(log) = compiled_shader.log {
//!                 log::info!("Shader compiler output: {}", log);
//!             }
//!             Ok(shader)
//!         } else {
//!             anyhow::bail!(
//!                 "Shader failed to compile: {}",
//!                 compiled_shader.log.as_deref().unwrap_or("Unknown error")
//!             );
//!         }
//!     }
//! }
//! ```

/// User-defined output of OpenGL's shader compiler, along with an info log.
pub struct ShaderCompilerOutput<Artifact> {
    pub artifact: Artifact,
    pub log: Option<String>,
}

/// Compile a shader defined as one or more `SourceChunk`s via a used-provided
/// shader compiler callback.
///
/// `source_chunks` is an iterator over `SourceChunks` to be compiled.
///
/// `Artifact` is a user-defined output of the shader compiler, e.g. `Option<GLuint>`.
///
/// `compiler_fn` is a function which, given a list of source strings
/// (which this function generates from `source_chunks`), creates a `ShaderCompilerOutput`.
pub fn compile_shader<'chunk, ChunksIter, Artifact, CompilerFn>(
    source_chunks: ChunksIter,
    compiler_fn: CompilerFn,
) -> ShaderCompilerOutput<Artifact>
where
    ChunksIter: Iterator<Item = &'chunk crate::source_chunk::SourceChunk>,
    CompilerFn: Fn(&[String]) -> ShaderCompilerOutput<Artifact>,
{
    struct FileAndLineOffset {
        file: String,
        line_offset: usize,
    }

    let (sources, file_and_line_offset): (Vec<String>, Vec<FileAndLineOffset>) = source_chunks
        .enumerate()
        .map(|(i, s)| {
            (
                if i == 0 {
                    s.source.clone()
                } else {
                    format!("#line 0 {}\n{}", i + 1, s.source)
                },
                FileAndLineOffset {
                    file: s.file.clone(),
                    line_offset: s.line_offset,
                },
            )
        })
        .unzip();

    let compiler_output = compiler_fn(&sources);

    lazy_static::lazy_static! {
        static ref INTEL_AMD_ERROR_RE: regex::Regex = regex::Regex::new(r"(?m)^ERROR:\s*(\d+):(\d+)").unwrap();
    }

    lazy_static::lazy_static! {
        static ref NV_ERROR_RE: regex::Regex = regex::Regex::new(r"(?m)^(\d+)\((\d+)\)\s*").unwrap();
    }

    let error_replacement = |captures: &regex::Captures| -> String {
        let chunk = captures[1].parse::<usize>().unwrap().max(1) - 1;
        let line = captures[2].parse::<usize>().unwrap();
        format!(
            "{}({})",
            file_and_line_offset[chunk].file,
            line + file_and_line_offset[chunk].line_offset
        )
    };

    let pretty_log = compiler_output.log.map(|log_str| {
        let log_str = INTEL_AMD_ERROR_RE.replace_all(&log_str, error_replacement);
        NV_ERROR_RE
            .replace_all(&log_str, error_replacement)
            .into_owned()
    });

    ShaderCompilerOutput {
        artifact: compiler_output.artifact,
        log: pretty_log,
    }
}
