use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;

///
pub trait IncludeProvider {
    fn get_include(&mut self, path: &str) -> Result<String, String>;
}

///
#[derive(PartialEq, Eq, Debug)]
pub struct SourceChunk {
    pub source: String,
    pub file: String,
    pub line_offset: usize,
}

///
pub fn process_file(
    path: &str,
    include_provider: &mut dyn IncludeProvider,
) -> Result<Vec<SourceChunk>, String> {
	let mut prior_includes = HashSet::new();
    let mut scanner = Scanner::new("", String::new(), &mut prior_includes, include_provider);
    scanner.include_child(path)?;
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

    fn include_child(&mut self, path: &str) -> Result<(), String> {
        if self.prior_includes.contains(path) {
            return Err("File recursively included".to_string() + path);
        }

        self.flush_current_chunk();

        let child_code = self.include_provider.get_include(path)?;

        self.prior_includes.insert(path.to_string());

        self.chunks.append(&mut {
            let mut child_scanner =
                Scanner::new(&child_code, path.to_string(), &mut self.prior_includes, self.include_provider);
            child_scanner.process_input()?;
            child_scanner.chunks
        });

        self.prior_includes.remove(path);

		Ok(())
    }

    fn process_input(&mut self) -> Result<(), String> {
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
								self.include_child(path)?;
							} else {
								return Err(format!("\"{}\" ({}): Could not parse include declaration.", self.this_file, c_line));
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
    use itertools::free::join;
    use std::collections::{HashMap, HashSet};

    struct DummyIncludeProvider;
    impl crate::IncludeProvider for DummyIncludeProvider {
        fn get_include(&mut self, path: &str) -> Result<String, String> {
            Ok(String::from("[") + path + "]")
        }
    }

    struct HashMapIncludeProvider(HashMap<String, String>);
    impl crate::IncludeProvider for HashMapIncludeProvider {
        fn get_include(&mut self, path: &str) -> Result<String, String> {
            Ok(self.0.get(path).unwrap().clone())
        }
    }

    fn preprocess_into_string(s: &str, include_provider: &mut crate::IncludeProvider) -> Result<String, String> {
		let mut prior_includes = HashSet::new();
        let mut scanner = crate::Scanner::new(s, "no-file".to_string(), &mut prior_includes, include_provider);
        scanner.process_input()?;
        Ok(join(
            scanner.chunks.into_iter().map(|chunk| chunk.source),
            "",
        ))
    }

    fn test_string(s: &str) -> String {
        match preprocess_into_string(s, &mut DummyIncludeProvider) {
			Ok(s) => s,
			Err(s) => s,
		}
    }

    #[test]
    fn ignore_unrecognized() {
        assert_eq!(test_string("*/ */ \t/ /"), "*/ */ \t/ /");
        assert_eq!(test_string("int foo;"), "int foo;");
        assert_eq!(
            test_string("#version 430\n#pragma stuff"),
            "#version 430\n#pragma stuff"
        );
    }

    #[test]
    fn basic_block_comment() {
        assert_eq!(test_string("foo /* bar */ baz"), "foo           baz");
        assert_eq!(test_string("foo /* /* bar */ baz"), "foo              baz");
    }

    #[test]
    fn basic_line_comment() {
        assert_eq!(test_string("foo // baz"), "foo ");
        assert_eq!(test_string("// foo /* bar */ baz"), "");
    }

    #[test]
    fn continued_line_comment() {
        assert_eq!(test_string("foo // baz\nbar"), "foo \nbar");
        assert_eq!(test_string("foo // baz\\\nbar"), "foo \n");
    }

    #[test]
    fn mixed_comments() {
        assert_eq!(
            test_string("/*\nfoo\n/*/\nbar\n//*/"),
            "  \n   \n   \nbar\n"
        );

        assert_eq!(
            test_string("//*\nfoo\n/*/\nbar\n//*/"),
            "\nfoo\n   \n   \n    "
        );
    }

    #[test]
    fn basic_preprocessor() {
        assert_eq!(test_string("#"), "#");
        assert_eq!(test_string("#in/**/clude"), "#in    clude");
        assert_eq!(test_string("#in\nclude"), "#in\nclude");
    }

    #[test]
    fn basic_include() {
        assert_eq!(test_string(r#"#include"foo""#), "[foo]");
        assert_eq!(test_string(r#"#include "foo""#), "[foo]");
        assert_eq!(test_string("#include <foo>"), "[foo]");
        assert_eq!(test_string("#include <foo/bar/baz>"), "[foo/bar/baz]");
        assert_eq!(test_string("#include <foo\\\nbar\\\nbaz>"), "[foobarbaz]");
        assert_eq!(test_string("#include <foo>//\n"), "[foo]\n");
        assert_eq!(test_string("# include <foo>"), "[foo]");
        assert_eq!(test_string("#  include <foo>"), "[foo]");
        assert_eq!(test_string("#/**/include <foo>"), "[foo]");
        assert_eq!(test_string("#include /**/ <foo>"), "[foo]");
    }

    #[test]
    fn multi_line_include() {
        assert_eq!(test_string("#inc\\\nlude <foo>"), "[foo]");
        assert_eq!(test_string("#inc\\\nlude"), "\"no-file\" (1): Could not parse include declaration.");
        assert_eq!(test_string("#\\\ninc\\\n\\\nlude <foo>"), "[foo]");
        assert_eq!(test_string("#\\\n   inc\\\n\\\nlude <foo>"), "[foo]");
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
            preprocess_into_string("#include <bar>", &mut include_provider),
            Ok("int bar;".to_string())
        );
        assert_eq!(
            preprocess_into_string("#include <foo>", &mut include_provider),
            Ok("double rainbow;\nint bar;\nint spam;\nint baz;\nvoid ham();".to_string())
        );

        assert_eq!(
            crate::process_file("foo", &mut include_provider),
            Ok(vec![
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
            ])
        );
    }

    #[test]
    fn include_err() {
        assert_eq!(test_string("#include"), "\"no-file\" (1): Could not parse include declaration.");
        assert_eq!(test_string("#include @"), "\"no-file\" (1): Could not parse include declaration.");
        assert_eq!(test_string("#include <foo"), "\"no-file\" (1): Could not parse include declaration.");
    }
}
