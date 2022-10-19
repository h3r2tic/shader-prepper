use std::collections::{HashMap, HashSet};

use crate::ResolvedIncludePath;

struct DummyIncludeProvider;
impl crate::IncludeProvider for DummyIncludeProvider {
    type IncludeContext = ();

    fn resolve_path(
        &self,
        path: &str,
        _context: &Self::IncludeContext,
    ) -> Result<crate::ResolvedInclude<Self::IncludeContext>, crate::BoxedIncludeProviderError>
    {
        Ok(crate::ResolvedInclude {
            resolved_path: crate::ResolvedIncludePath(path.to_owned()),
            context: (),
        })
    }

    fn get_include(
        &mut self,
        resolved: &ResolvedIncludePath,
    ) -> Result<String, crate::BoxedIncludeProviderError> {
        Ok(format!("[{}]", resolved.0))
    }
}

struct HashMapIncludeProvider(HashMap<String, String>);
impl crate::IncludeProvider for HashMapIncludeProvider {
    type IncludeContext = ();

    fn resolve_path(
        &self,
        path: &str,
        _context: &Self::IncludeContext,
    ) -> Result<crate::ResolvedInclude<Self::IncludeContext>, crate::BoxedIncludeProviderError>
    {
        Ok(crate::ResolvedInclude {
            resolved_path: crate::ResolvedIncludePath(path.to_owned()),
            context: (),
        })
    }

    fn get_include(
        &mut self,
        resolved: &ResolvedIncludePath,
    ) -> Result<String, crate::BoxedIncludeProviderError> {
        Ok(self.0.get(&resolved.0).unwrap().clone())
    }
}

fn preprocess_into_string<IncludeContext: Clone>(
    s: &str,
    include_provider: &mut dyn crate::IncludeProvider<IncludeContext = IncludeContext>,
    include_context: IncludeContext,
) -> Result<String, crate::PrepperError> {
    let mut prior_includes = HashSet::new();
    let mut skip_includes = HashSet::new();

    let mut scanner = crate::Scanner::new(
        s,
        "no-file".to_string(),
        &mut prior_includes,
        &mut skip_includes,
        include_provider,
        include_context,
    );
    scanner.process_input()?;
    Ok(scanner
        .into_chunks()
        .into_iter()
        .map(|chunk| chunk.source)
        .collect::<Vec<_>>()
        .join(""))
}

fn test_string(s: &str, s2: &str) {
    match preprocess_into_string(s, &mut DummyIncludeProvider, ()) {
        Ok(r) => assert_eq!(r, s2.to_string()),
        val => panic!("{:?}", val),
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
    match preprocess_into_string("#inc\\\nlude", &mut DummyIncludeProvider, ()) {
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
        preprocess_into_string("#include <bar>", &mut include_provider, ()).unwrap(),
        "int bar;"
    );
    assert_eq!(
        preprocess_into_string("#include <foo>", &mut include_provider, ()).unwrap(),
        "double rainbow;\nint bar;\nint spam;\nint baz;\nvoid ham();"
    );

    assert_eq!(
        crate::process_file("foo", &mut include_provider, ()).unwrap(),
        vec![
            crate::SourceChunk {
                file: "foo".to_string(),
                line_offset: 0,
                source: "double rainbow;\n".to_string(),
                context: (),
            },
            crate::SourceChunk {
                file: "bar".to_string(),
                line_offset: 0,
                source: "int bar;".to_string(),
                context: (),
            },
            crate::SourceChunk {
                file: "foo".to_string(),
                line_offset: 1,
                source: "\nint spam;\n".to_string(),
                context: (),
            },
            crate::SourceChunk {
                file: "baz".to_string(),
                line_offset: 0,
                source: "int baz;".to_string(),
                context: (),
            },
            crate::SourceChunk {
                file: "foo".to_string(),
                line_offset: 3,
                source: "\nvoid ham();".to_string(),
                context: (),
            },
        ]
    );
}

#[test]
fn pragma_once() {
    let mut recursive_include_provider = HashMapIncludeProvider(
        [
            ("foo", "#pragma once\nthis_is_foo"),
            ("bar", "#include <foo>\n#include <foo>\n#include <foo>"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect(),
    );

    assert_eq!(
        preprocess_into_string("#include <bar>", &mut recursive_include_provider, ())
            .unwrap()
            .trim(),
        "this_is_foo"
    );
}

#[test]
fn include_err() {
    match preprocess_into_string("#include", &mut DummyIncludeProvider, ()) {
        Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
        val => panic!("{:?}", val),
    }

    match preprocess_into_string("#include @", &mut DummyIncludeProvider, ()) {
        Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
        val => panic!("{:?}", val),
    }

    match preprocess_into_string("#include <foo", &mut DummyIncludeProvider, ()) {
        Err(crate::PrepperError::ParseError { file: _, line: 1 }) => (),
        val => panic!("{:?}", val),
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

    match &preprocess_into_string("#include <foo>", &mut recursive_include_provider, ()) {
        Err(crate::PrepperError::RecursiveInclude {
            file: fname,
            from: fsrc,
            from_line: 1,
        }) if fname == "foo" && fsrc == "baz" => (),
        val => panic!("{:?}", val),
    }
}

struct FileIncludeProvider;
impl crate::IncludeProvider for FileIncludeProvider {
    type IncludeContext = ();

    fn resolve_path(
        &self,
        path: &str,
        _context: &Self::IncludeContext,
    ) -> Result<crate::ResolvedInclude<Self::IncludeContext>, crate::BoxedIncludeProviderError>
    {
        Ok(crate::ResolvedInclude {
            resolved_path: crate::ResolvedIncludePath(path.to_owned()),
            context: (),
        })
    }

    fn get_include(
        &mut self,
        resolved: &ResolvedIncludePath,
    ) -> Result<String, crate::BoxedIncludeProviderError> {
        Ok(std::fs::read_to_string(&resolved.0)?)
    }
}

#[test]
fn include_file() {
    assert!(preprocess_into_string("src/lib.rs", &mut FileIncludeProvider, ()).is_ok());
}
