# shader-prepper

**shader-prepper** is a shader include parser and crawler. It is mostly aimed at GLSL
which doesn't provide include directive support out of the box.

This crate does not implement a full C-like preprocessor, only `#include` scanning.
Other directives are instead copied into the expanded code, so they can be subsequently
handled by the shader compiler.

The API supports user-driven include file providers, which enable custom
virtual file systems, include paths, and allow build systems to track dependencies.

Source files are not concatenated together, but returned as a Vec of [`SourceChunk`].
If a single string is needed, a `join` over the source strings can be used.
Otherwise, the individual chunks can be passed to the graphics API, and source info
contained within `SourceChunk` can then remap the compiler's errors back to
the original code.

## Example

```rust
use failure;

struct FileIncludeProvider;
impl shader_prepper::IncludeProvider for FileIncludeProvider {
    fn get_include(&mut self, path: &str) -> Result<String, failure::Error> {
        std::fs::read_to_string(path).map_err(|e| failure::format_err!("{}", e))
    }
}

// ...

let chunks = shader_prepper::process_file("myfile.glsl", &mut FileIncludeProvider);
```

License: MIT
