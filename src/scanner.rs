use std::collections::HashSet;
use std::iter::Peekable;
use std::str::Chars;

use crate::{IncludeProvider, PrepperError, SourceChunk};

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

type ResolvedPathString = String;

// Inspired by JayKickliter/monkey
pub struct Scanner<'input, 'provider, 'state, IncludeContext> {
    include_provider: &'provider mut dyn IncludeProvider<IncludeContext = IncludeContext>,
    include_context: IncludeContext,
    input_iter: Peekable<LocationTracking<Chars<'input>>>,
    this_file: ResolvedPathString,
    prior_includes: &'state mut HashSet<ResolvedPathString>,
    skip_includes: &'state mut HashSet<ResolvedPathString>,
    chunks: Vec<SourceChunk<IncludeContext>>,
    current_chunk: String,
    current_chunk_first_line: u32,
}

impl<'input, 'provider, 'state, IncludeContext> Scanner<'input, 'provider, 'state, IncludeContext>
where
    IncludeContext: Clone,
{
    pub fn new(
        input: &'input str,
        this_file: String,
        prior_includes: &'state mut HashSet<String>,
        skip_includes: &'state mut HashSet<String>,
        include_provider: &'provider mut dyn IncludeProvider<IncludeContext = IncludeContext>,
        include_context: IncludeContext,
    ) -> Scanner<'input, 'provider, 'state, IncludeContext> {
        Scanner {
            include_provider,
            include_context,
            input_iter: LocationTracking {
                iter: input.chars(),
                line: 1,
            }
            .peekable(),
            this_file,
            prior_includes,
            skip_includes,
            chunks: Vec::new(),
            current_chunk: String::new(),
            current_chunk_first_line: 1,
        }
    }

    pub fn into_chunks(self) -> Vec<SourceChunk<IncludeContext>> {
        self.chunks
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

                    self.input_iter = skip_block_comment(self.input_iter.clone()).1;
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
    ) -> Option<(String, Peekable<LocationTracking<Chars<'input>>>)> {
        peek_preprocessor_ident(self.input_iter.clone())
    }

    fn flush_current_chunk(&mut self) {
        if !self.current_chunk.is_empty() {
            self.chunks.push(SourceChunk {
                file: self.this_file.clone(),
                line_offset: (self.current_chunk_first_line - 1) as usize,
                source: self.current_chunk.clone(),
                context: self.include_context.clone(),
            });
            self.current_chunk.clear();
        }

        if let Some(&(line, _)) = self.peek_char() {
            self.current_chunk_first_line = line;
        }
    }

    pub fn include_child(&mut self, path: &str, included_on_line: u32) -> Result<(), PrepperError> {
        if self.prior_includes.contains(path) {
            return Err(PrepperError::RecursiveInclude {
                file: path.to_string(),
                from: self.this_file.clone(),
                from_line: included_on_line as usize,
            });
        }

        self.flush_current_chunk();

        let child = self
            .include_provider
            .resolve_path(path, &self.include_context)
            .map_err(|e| PrepperError::IncludeProviderError {
                file: path.to_string(),
                cause: e,
            })?;

        if self.skip_includes.contains(&child.resolved_path.0) {
            return Ok(());
        }

        let child_code = self
            .include_provider
            .get_include(&child.resolved_path)
            .map_err(|e| PrepperError::IncludeProviderError {
                file: path.to_string(),
                cause: e,
            })?;

        self.prior_includes.insert(path.to_string());

        self.chunks.append(&mut {
            let mut child_scanner = Scanner::new(
                &child_code,
                child.resolved_path.0.to_string(),
                self.prior_includes,
                self.skip_includes,
                self.include_provider,
                child.context,
            );
            child_scanner.process_input()?;
            child_scanner.chunks
        });

        self.prior_includes.remove(path);

        Ok(())
    }

    pub fn process_input(&mut self) -> Result<(), PrepperError> {
        while let Some((c_line, c)) = self.read_char() {
            match c {
                '/' => {
                    let next = self.peek_char();

                    if let Some(&(_, '*')) = next {
                        let _ = self.read_char();
                        self.current_chunk.push_str("  ");
                        let (white, it) = skip_block_comment(self.input_iter.clone());

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
                        } else if "pragma" == preprocessor_ident.0 {
                            let next_ident = peek_preprocessor_ident(preprocessor_ident.1);

                            match next_ident {
                                Some((pragma_type, next_iter)) if pragma_type == "once" => {
                                    self.input_iter = next_iter;
                                    self.skip_whitespace_until_eol();

                                    self.skip_includes.insert(self.this_file.clone());
                                }
                                _ => {
                                    self.current_chunk.push(c);
                                }
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

fn skip_block_comment(
    mut it: Peekable<LocationTracking<Chars<'_>>>,
) -> (String, Peekable<LocationTracking<Chars<'_>>>) {
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

fn peek_preprocessor_ident(
    mut it: Peekable<LocationTracking<Chars<'_>>>,
) -> Option<(String, Peekable<LocationTracking<Chars<'_>>>)> {
    let mut token = String::new();

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

                it = skip_block_comment(it).1;
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
