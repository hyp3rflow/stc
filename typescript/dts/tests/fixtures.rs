#![recursion_limit = "256"]
#![feature(vec_remove_item)]
#![feature(box_syntax)]
#![feature(box_patterns)]
#![feature(test)]

extern crate test;

use anyhow::{Context, Error};
use std::{
    cmp::Ordering::Equal,
    env,
    fs::{canonicalize, File},
    io::Read,
    path::Path,
    process::Command,
    sync::Arc,
};
use swc_common::{input::SourceFileInput, FileName, SourceFile};
use swc_ecma_ast::{Module, TsKeywordTypeKind, TsLit, TsLitType, TsType, TsUnionType};
use swc_ecma_codegen::{text_writer::JsWriter, Emitter};
use swc_ecma_parser::{lexer::Lexer, JscTarget, Parser, StringInput, Syntax, TsConfig};
use swc_ecma_utils::drop_span;
use swc_ecma_visit::FoldWith;
use swc_ts_checker::Lib;
use swc_ts_dts::generate_dts;
use test::{test_main, DynTestFn, ShouldPanic::No, TestDesc, TestDescAndFn, TestName, TestType};
use testing::{NormalizedOutput, StdErr};

#[test]
fn fixtures_test() {
    let args: Vec<_> = env::args().collect();
    let mut tests = Vec::new();
    add_fixture_tests(&mut tests).unwrap();
    test_main(&args, tests, Default::default());
}

fn add_fixture_tests(tests: &mut Vec<TestDescAndFn>) -> Result<(), Error> {
    let root = {
        let mut root = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        root.push("tests");
        root.push("fixtures");

        root
    };

    for entry in ignore::WalkBuilder::new(&root)
        .ignore(false)
        .git_exclude(false)
        .git_global(false)
        .git_ignore(false)
        .hidden(false)
        .build()
    {
        let entry = entry.context("entry?")?;

        if entry.file_name().to_string_lossy().ends_with(".d.ts") {
            continue;
        }

        let is_ts = entry.file_name().to_string_lossy().ends_with(".ts")
            || entry.file_name().to_string_lossy().ends_with(".tsx");
        if entry.file_type().unwrap().is_dir() || !is_ts {
            continue;
        }

        let file_name = entry
            .path()
            .strip_prefix(&root)
            .expect("failed to strip prefix")
            .to_str()
            .unwrap()
            .to_string();

        let input = {
            let mut buf = String::new();
            File::open(entry.path())?.read_to_string(&mut buf)?;

            // Disable tests for dynamic import
            if buf.contains("import(") {
                continue;
            }

            buf
        };

        let test_name = file_name.replace("/", "::");

        // Prevent regression
        let ignore = test_name.contains("::.") || test_name.starts_with(".");

        let name = format!("{}", test_name);

        add_test(tests, name, ignore, move || {
            println!("----- Input -----\n{}", input);
            do_test(entry.path()).unwrap();
        });
    }

    Ok(())
}

fn do_test(file_name: &Path) -> Result<(), StdErr> {
    let file_name = canonicalize(file_name).unwrap();
    let fname = file_name.display().to_string();
    let (expected_code, expected) = get_correct_dts(&file_name);
    let expected = drop_span(expected.fold_with(&mut Normalizer));
    println!("---------- Expected ----------\n{}", expected_code);

    testing::Tester::new()
        .print_errors(|cm, handler| {
            let handler = Arc::new(handler);

            let checker = swc_ts_checker::Checker::new(
                Default::default(),
                cm.clone(),
                handler.clone(),
                Lib::load("es2019.full"),
                Default::default(),
                TsConfig {
                    tsx: fname.contains("tsx"),
                    decorators: true,
                    ..Default::default()
                },
                JscTarget::Es5,
            );

            let info = checker.check(Arc::new(file_name.clone().into()));
            let errors = ::swc_ts_checker::errors::Error::flatten(info.1.errors.into());

            let expected = {
                let mut buf = vec![];
                {
                    let mut emitter = Emitter {
                        cfg: Default::default(),
                        comments: None,
                        cm: cm.clone(),
                        wr: box JsWriter::new(cm.clone(), "\n", &mut buf, None),
                    };

                    emitter
                        .emit_module(&expected)
                        .context("failed to emit module")
                        .unwrap();
                }
                String::from_utf8(buf).unwrap()
            };

            let dts = generate_dts(info.0, info.1.exports).fold_with(&mut Normalizer);

            let generated = {
                let mut buf = vec![];
                {
                    let mut emitter = Emitter {
                        cfg: Default::default(),
                        comments: None,
                        cm: cm.clone(),
                        wr: box JsWriter::new(cm.clone(), "\n", &mut buf, None),
                    };

                    emitter
                        .emit_module(&dts)
                        .context("failed to emit module")
                        .unwrap();
                }
                String::from_utf8(buf).unwrap()
            };

            println!("---------- Generated ----------\n{}", generated);
            let generated = NormalizedOutput::from(generated);
            let expected = NormalizedOutput::from(expected);
            if generated == expected {
                return Ok(());
            }
            println!("{}", testing::diff(&generated, &expected));

            checker.run(|| {
                for e in errors {
                    e.emit(&handler);
                }
            });

            let expected = parse_dts(&expected);
            let generated = parse_dts(&generated);

            if expected == generated {
                return Ok(());
            }

            Err(())
        })
        .expect("failed to check");

    Ok(())
}

fn parse_dts(src: &str) -> Module {
    ::testing::run_test2(false, |cm, handler| {
        let fm = cm.new_source_file(FileName::Anon, src.to_string());

        let mut lexer = Lexer::new(
            Syntax::Typescript(TsConfig {
                dts: true,
                ..Default::default()
            }),
            JscTarget::Es2020,
            StringInput::from(&*fm),
            None,
        );

        let mut parser = Parser::new_from(lexer);
        let module = parser.parse_module().unwrap();
        Ok(drop_span(module))
    })
    .unwrap()
}

fn get_correct_dts(path: &Path) -> (Arc<String>, Module) {
    testing::run_test2(false, |cm, handler| {
        let dts_file = path.parent().unwrap().join(format!(
            "{}.d.ts",
            path.file_stem().unwrap().to_string_lossy()
        ));

        if !dts_file.exists() {
            let mut c = Command::new(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("node_modules")
                    .join(".bin")
                    .join("tsc"),
            );
            c.arg(path)
                .arg("--jsx")
                .arg("preserve")
                .arg("-d")
                .arg("--emitDeclarationOnly")
                .arg("--target")
                .arg("es2019")
                .arg("--lib")
                .arg("es2019");
            let output = c.output().unwrap();

            if !output.status.success() {
                panic!(
                    "Failed to get correct dts file\n{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
        }

        let content = {
            let mut buf = String::new();
            let mut f = File::open(&dts_file).expect("failed to open generated .d.ts file");
            f.read_to_string(&mut buf).unwrap();
            buf.replace("export {};", "")
        };

        let fm = cm.new_source_file(FileName::Real(dts_file), content);

        let mut p = Parser::new(
            Syntax::Typescript(TsConfig {
                tsx: true,
                decorators: true,
                dynamic_import: true,
                dts: true,
                no_early_errors: true,
            }),
            SourceFileInput::from(&*fm),
            None,
        );

        let m = p
            .parse_typescript_module()
            .map_err(|e| e.into_diagnostic(&handler).emit())?;

        Ok((fm.src.clone(), m))
    })
    .unwrap()
}

fn add_test<F: FnOnce() + Send + 'static>(
    tests: &mut Vec<TestDescAndFn>,
    name: String,
    ignore: bool,
    f: F,
) {
    tests.push(TestDescAndFn {
        desc: TestDesc {
            test_type: TestType::UnitTest,
            name: TestName::DynTestName(name),
            ignore,
            should_panic: No,
            allow_fail: false,
        },
        testfn: DynTestFn(box f),
    });
}

struct Normalizer;

/// Sorts the type.
impl swc_ecma_visit::Fold for Normalizer {
    fn fold_ts_types(&mut self, mut types: Vec<Box<TsType>>) -> Vec<Box<TsType>> {
        fn kwd_rank(kind: TsKeywordTypeKind) -> u8 {
            match kind {
                TsKeywordTypeKind::TsNumberKeyword => 0,
                TsKeywordTypeKind::TsStringKeyword => 1,
                TsKeywordTypeKind::TsBooleanKeyword => 2,
                _ => 4,
            }
        }

        types.sort_by(|a, b| match (&**a, &**b) {
            (&TsType::TsKeywordType(ref a), &TsType::TsKeywordType(ref b)) => {
                kwd_rank(a.kind).cmp(&kwd_rank(b.kind))
            }
            (
                &TsType::TsLitType(TsLitType {
                    lit: TsLit::Str(ref a),
                    ..
                }),
                &TsType::TsLitType(TsLitType {
                    lit: TsLit::Str(ref b),
                    ..
                }),
            ) => a.value.cmp(&b.value),

            _ => Equal,
        });

        types
    }
}
