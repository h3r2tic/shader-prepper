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
//! use failure;
//!
//! struct FileIncludeProvider;
//! impl shader_prepper::IncludeProvider for FileIncludeProvider {
//!     fn get_include(&mut self, path: &str) -> Result<String, failure::Error> {
//!         std::fs::read_to_string(path).map_err(|e| failure::format_err!("{}", e))
//!     }
//! }
//!
//! // ...
//!
//! let chunks = shader_prepper::process_file("myfile.glsl", &mut FileIncludeProvider);
//! ```

#[macro_use]
extern crate failure;

use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;

use failure::Error;

#[derive(Debug, Fail)]
pub enum PrepperError {
    /// Any error reported by the user-supplied `IncludeProvider`
    #[fail(
        display = "include provider error: \"{}\" when trying to include {}",
        cause, file
    )]
    IncludeProviderError {
        file: String,
        #[cause]
        cause: Error,
    },

    /// Recursively included file, along with information about where it was encountered
    #[fail(
        display = "file {} is recursively included; triggered in {} ({})",
        file, from, from_line
    )]
    RecursiveInclude {
        /// File which was included recursively
        file: String,

        /// File which included the recursively included one
        from: String,

        /// Line in the `from` file on which the include happened
        from_line: usize,
    },

    /// Error parsing an include directive
    #[fail(display = "parse error: {} ({})", file, line)]
    ParseError { file: String, line: usize },
}

/// User-supplied include reader
pub trait IncludeProvider {
    fn get_include(&mut self, path: &str) -> Result<String, Error>;
}

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

/// Process a single file, and then any code recursively referenced.
///
/// `include_provider` is used to read all of the files, including the one at `file_path`.
pub fn process_file(
    file_path: &str,
    include_provider: &mut dyn IncludeProvider,
) -> Result<Vec<SourceChunk>, Error> {
    let mut prior_includes = HashSet::new();
    let mut scanner = Scanner::new("", String::new(), &mut prior_includes, include_provider);
    scanner.include_child(file_path, 1)?;
    Ok(scanner.chunks)
}

#[derive(Clone)]
struct LocationTracking<I> {
    iter: I,
    line: u32,
}

impl<I> Iterator for LocationTracking<I>
where
    I: Iterator<Item = char>,
{
    type Item = (u32, <I as Iterator>::Item);

    #[inline]
    fn next(&mut self) -> Option<(u32, <I as Iterator>::Item)> {
        self.iter.next().map(|a| {
            let nl = a == '\n';
            let ret = (self.line, a);
            // Possible undefined overflow.
            if nl {
                self.line += 1;
            }
            ret
        })
    }
}

// Inspired by JayKickliter/monkey
struct Scanner<'a, 'b, 'c> {
    include_provider: &'b mut dyn IncludeProvider,
    input_iter: Peekable<LocationTracking<Chars<'a>>>,
    this_file: String,
    prior_includes: &'c mut HashSet<String>,
    chunks: Vec<SourceChunk>,
    current_chunk: String,
    current_chunk_first_line: u32,
}

impl<'a, 'b, 'c> Scanner<'a, 'b, 'c> {
    fn new(
        input: &'a str,
        this_file: String,
        prior_includes: &'c mut HashSet<String>,
        include_provider: &'b mut dyn IncludeProvider,
    ) -> Scanner<'a, 'b, 'c> {
        Scanner {
            include_provider,
            input_iter: LocationTracking {
                iter: input.chars(),
                line: 1,
            }
            .peekable(),
            this_file,
            prior_includes,
            chunks: Vec::new(),
            current_chunk: String::new(),
            current_chunk_first_line: 1,
        }
    }

    fn read_char(&mut self) -> Option<(u32, char)> {
        self.input_iter.next()
    }

    fn peek_char(&mut self) -> Option<&(u32, char)> {
        self.input_iter.peek()
    }

    fn skip_whitespace_until_eol(&mut self) {
        while let Some(&(_, c)) = self.peek_char() {
            if c == '\n' {
                break;
            } else if c.is_whitespace() {
                let _ = self.read_char();
            } else if c == '\\' {
                let mut peek_next = self.input_iter.clone();
                let _ = peek_next.next();
                if let Some(&(_, '\n')) = peek_next.peek() {
                    let _ = self.read_char();
                    let _ = self.read_char();
                } else {
                    break;
                }
            } else if c == '/' {
                let mut next_peek = self.input_iter.clone();
                let _ = next_peek.next();

                if let Some(&(_, '*')) = next_peek.peek() {
                    // Block comment. Skip it.
                    let _ = self.read_char();
                    let _ = self.read_char();

                    self.input_iter = Self::skip_block_comment(self.input_iter.clone()).1;
                }
            } else {
                break;
            }
        }
    }

    fn read_string(&mut self, right_delim: char) -> Option<String> {
        let mut s = String::new();

        while let Some(&(_, c)) = self.peek_char() {
            if c == '\n' {
                break;
            } else if c == '\\' {
                let _ = self.read_char();
                let _ = self.read_char();
            } else if c == right_delim {
                let _ = self.read_char();
                return Some(s);
            } else {
                s.push(c);
                let _ = self.read_char();
            }
        }

        None
    }

    fn skip_block_comment(
        mut it: Peekable<LocationTracking<Chars<'a>>>,
    ) -> (String, Peekable<LocationTracking<Chars<'a>>>) {
        let mut s = String::new();

        while let Some((_, c)) = it.next() {
            if c == '*' {
                s.push(' ');
                if let Some(&(_, '/')) = it.peek() {
                    let _ = it.next();
                    s.push(' ');
                    break;
                }
            } else if c == '\n' {
                s.push('\n');
            } else {
                s.push(' ');
            }
        }

        (s, it)
    }

    fn skip_line(&mut self) {
        while let Some((_, c)) = self.read_char() {
            if c == '\n' {
                self.current_chunk.push('\n');
                break;
            } else if c == '\\' {
                if let Some((_, '\n')) = self.read_char() {
                    self.current_chunk.push('\n');
                }
            }
        }
    }

    fn peek_preprocessor_ident(
        &mut self,
    ) -> Option<(String, Peekable<LocationTracking<Chars<'a>>>)> {
        let mut token = String::new();
        let mut it = self.input_iter.clone();

        while let Some(&(_, c)) = it.peek() {
            if '\n' == c || '\r' == c {
                break;
            } else if c.is_alphabetic() {
                let _ = it.next();
                token.push(c);
            } else if c.is_whitespace() {
                if !token.is_empty() {
                    // Already found some chars, and this ends the identifier
                    break;
                } else {
                    // Still haven't found anything. Continue scanning.
                    let _ = it.next();
                }
            } else if '\\' == c {
                let _ = it.next();
                let next = it.next();

                if let Some((_, '\n')) = next {
                    // Continue scanning on next line
                    continue;
                } else if let (Some((_, '\r')), Some(&(_, '\n'))) = (next, it.peek()) {
                    // ditto, but Windows-special
                    let _ = it.next();
                    continue;
                } else {
                    // Unrecognized escape sequence. Abort.
                    return None;
                }
            } else if '/' == c {
                if !token.is_empty() {
                    // Already found some chars, and this ends the identifier
                    break;
                }

                let mut next_peek = it.clone();
                let _ = next_peek.next();

                if let Some(&(_, '*')) = next_peek.peek() {
                    // Block comment. Skip it.
                    let _ = it.next();
                    let _ = it.next();

                    it = Self::skip_block_comment(it).1;
                } else {
                    // Something other than a block comment. End the identifier.
                    break;
                }
            } else {
                // Some other character. This finishes the identifier.
                break;
            }
        }

        Some((token, it))
    }

    fn flush_current_chunk(&mut self) {
        if !self.current_chunk.is_empty() {
            self.chunks.push(SourceChunk {
                file: self.this_file.clone(),
                line_offset: (self.current_chunk_first_line - 1) as usize,
                source: self.current_chunk.clone(),
            });
            self.current_chunk.clear();
        }

        if let Some(&(line, _)) = self.peek_char() {
            self.current_chunk_first_line = line;
        }
    }

    fn include_child(&mut self, path: &str, included_on_line: u32) -> Result<(), PrepperError> {
        if self.prior_includes.contains(path) {
            return Err(PrepperError::RecursiveInclude {
                file: path.to_string(),
                from: self.this_file.clone(),
                from_line: included_on_line as usize,
            });
        }

        self.flush_current_chunk();

        let child_code = self.include_provider.get_include(path).map_err(|e| {
            PrepperError::IncludeProviderError {
                file: path.to_string(),
                cause: e,
            }
        })?;

        self.prior_includes.insert(path.to_string());

        self.chunks.append(&mut {
            let mut child_scanner = Scanner::new(
                &child_code,
                path.to_string(),
                &mut self.prior_includes,
                self.include_provider,
            );
            child_scanner.process_input()?;
            child_scanner.chunks
        });

        self.prior_includes.remove(path);

        Ok(())
    }

    fn process_input(&mut self) -> Result<(), PrepperError> {
        while let Some((c_line, c)) = self.read_char() {
            match c {
                '/' => {
                    let next = self.peek_char();

                    if let Some(&(_, '*')) = next {
                        let _ = self.read_char();
                        self.current_chunk.push_str("  ");
                        let (white, it) = Self::skip_block_comment(self.input_iter.clone());

                        self.input_iter = it;
                        self.current_chunk.push_str(&white);
                    } else if let Some(&(_, '/')) = next {
                        let _ = self.read_char();
                        self.skip_line();
                    } else {
                        self.current_chunk.push(c);
                    }
                }
                '#' => {
                    if let Some(preprocessor_ident) = self.peek_preprocessor_ident() {
                        if "include" == preprocessor_ident.0 {
                            self.input_iter = preprocessor_ident.1;
                            self.skip_whitespace_until_eol();

                            let left_delim = self.read_char();

                            let right_delim = match left_delim {
                                Some((_, '"')) => Some('"'),
                                Some((_, '<')) => Some('>'),
                                _ => None,
                            };

                            let path = right_delim
                                .map(|right_delim| self.read_string(right_delim))
                                .unwrap_or_default();

                            if let Some(ref path) = path {
                                self.include_child(path, c_line)?;
                            } else {
                                return Err(PrepperError::ParseError {
                                    file: self.this_file.clone(),
                                    line: c_line as usize,
                                });
                            }
                        } else {
                            self.current_chunk.push(c);
                        }
                    } else {
                        self.current_chunk.push(c);
                    }
                }
                _ => {
                    self.current_chunk.push(c);
                }
            }
        }

        self.flush_current_chunk();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    struct DummyIncludeProvider;
    impl crate::IncludeProvider for DummyIncludeProvider {
        fn get_include(&mut self, path: &str) -> Result<String, crate::Error> {
            Ok(String::from("[") + path + "]")
        }
    }

    struct HashMapIncludeProvider(HashMap<String, String>);
    impl crate::IncludeProvider for HashMapIncludeProvider {
        fn get_include(&mut self, path: &str) -> Result<String, crate::Error> {
            Ok(self.0.get(path).unwrap().clone())
        }
    }

    fn preprocess_into_string(
        s: &str,
        include_provider: &mut crate::IncludeProvider,
    ) -> Result<String, crate::PrepperError> {
        let mut prior_includes = HashSet::new();
        let mut scanner = crate::Scanner::new(
            s,
            "no-file".to_string(),
            &mut prior_includes,
            include_provider,
        );
        scanner.process_input()?;
        Ok(scanner.chunks.into_iter().map(|chunk| chunk.source).collect::<Vec<_>>().join(""))
    }

    fn test_string(s: &str, s2: &str) {
        match preprocess_into_string(s, &mut DummyIncludeProvider) {
            Ok(r) => assert_eq!(r, s2.to_string()),
            val @ _ => panic!("{:?}", val),
        };
    }

    #[test]
    fn ignore_unrecognized() {
        test_string("*/ */ \t/ /", "*/ */ \t/ /");
        test_string("int foo;", "int foo;");
        test_string("#version 430\n#pragma stuff", "#version 430\n#pragma stuff");
    }

    #[test]
    fn basic_block_comment() {
        test_string("foo /* bar */ baz", "foo           baz");
        test_string("foo /* /* bar */ baz", "foo              baz");
    }

    #[test]
    fn basic_line_comment() {
        test_string("foo // baz", "foo ");
        test_string("// foo /* bar */ baz", "");
    }

    #[test]
    fn continued_line_comment() {
        test_string("foo // baz\nbar", "foo \nbar");
        test_string("foo // baz\\\nbar", "foo \n");
    }

    #[test]
    fn mixed_comments() {
        test_string("/*\nfoo\n/*/\nbar\n//*/", "  \n   \n   \nbar\n");
        test_string("//*\nfoo\n/*/\nbar\n//*/", "\nfoo\n   \n   \n    ");
    }

    #[test]
    fn basic_preprocessor() {
        test_string("#", "#");
        test_string("#in/**/clude", "#in    clude");
        test_string("#in\nclude", "#in\nclude");
    }

    #[test]
    fn basic_include() {
        test_string(r#"#include"foo""#, "[foo]");
        test_string(r#"#include "foo""#, "[foo]");
        test_string("#include <foo>", "[foo]");
        test_string("#include <foo/bar/baz>", "[foo/bar/baz]");
        test_string("#include <foo\\\nbar\\\nbaz>", "[foobarbaz]");
        test_string("#include <foo>//\n", "[foo]\n");
        test_string("# include <foo>", "[foo]");
        test_string("#  include <foo>", "[foo]");
        test_string("#/**/include <foo>", "[foo]");
        test_string("#include /**/ <foo>", "[foo]");
    }

    #[test]
    fn multi_line_include() {
        match preprocess_into_string("#inc\\\nlude", &mut DummyIncludeProvider) {
            Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
            _ => panic!(),
        }

        test_string("#inc\\\nlude <foo>", "[foo]");
        test_string("#\\\ninc\\\n\\\nlude <foo>", "[foo]");
        test_string("#\\\n   inc\\\n\\\nlude <foo>", "[foo]");
    }

    #[test]
    fn multi_level_include() {
        let mut include_provider = HashMapIncludeProvider(
            [
                (
                    "foo",
                    "double rainbow;\n#include <bar>\nint spam;\n#include <baz>\nvoid ham();",
                ),
                ("bar", "int bar;"),
                ("baz", "int baz;"),
            ]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        );

        assert_eq!(
            preprocess_into_string("#include <bar>", &mut include_provider).unwrap(),
            "int bar;"
        );
        assert_eq!(
            preprocess_into_string("#include <foo>", &mut include_provider).unwrap(),
            "double rainbow;\nint bar;\nint spam;\nint baz;\nvoid ham();"
        );

        assert_eq!(
            crate::process_file("foo", &mut include_provider).unwrap(),
            vec![
                crate::SourceChunk {
                    file: "foo".to_string(),
                    line_offset: 0,
                    source: "double rainbow;\n".to_string()
                },
                crate::SourceChunk {
                    file: "bar".to_string(),
                    line_offset: 0,
                    source: "int bar;".to_string()
                },
                crate::SourceChunk {
                    file: "foo".to_string(),
                    line_offset: 1,
                    source: "\nint spam;\n".to_string()
                },
                crate::SourceChunk {
                    file: "baz".to_string(),
                    line_offset: 0,
                    source: "int baz;".to_string()
                },
                crate::SourceChunk {
                    file: "foo".to_string(),
                    line_offset: 3,
                    source: "\nvoid ham();".to_string()
                },
            ]
        );
    }

    #[test]
    fn include_err() {
        match preprocess_into_string("#include", &mut DummyIncludeProvider) {
            Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
            val @ _ => panic!("{:?}", val),
        }

        match preprocess_into_string("#include @", &mut DummyIncludeProvider) {
            Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
            val @ _ => panic!("{:?}", val),
        }

        match preprocess_into_string("#include <foo", &mut DummyIncludeProvider) {
            Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
            val @ _ => panic!("{:?}", val),
        }

        let mut recursive_include_provider = HashMapIncludeProvider(
            [
                ("foo", "#include <bar>"),
                ("bar", "#include <baz>"),
                ("baz", "#include <foo>"),
            ]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
        );

        match &preprocess_into_string("#include <foo>", &mut recursive_include_provider) {
            Err(crate::PrepperError::RecursiveInclude {
                file: fname @ _,
                from: fsrc @ _,
                from_line: 1,
            }) if fname == "foo" && fsrc == "baz" => (),
            val @ _ => panic!("{:?}", val),
        }
    }

    struct FileIncludeProvider;
    impl crate::IncludeProvider for FileIncludeProvider {
        fn get_include(&mut self, path: &str) -> Result<String, failure::Error> {
            std::fs::read_to_string(path).map_err(|e| format_err!("{}", e))
        }
    }

    #[test]
    fn include_file() {
        assert!(preprocess_into_string("src/lib.rs", &mut FileIncludeProvider).is_ok());
    }
}
