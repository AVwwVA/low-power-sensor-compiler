mod ast;
mod codegen;
mod config;
mod diagnostics;
mod lexer;
mod lowerer;
mod optimizer;
mod parser;
mod task_ir;
mod typechecker;
mod types;

use chumsky::Parser;
use clap::Parser as ClapParser;
use config::CompilerConfig;
use diagnostics::{SourceFile, concise_parse_error_message};
use lexer::Lexer;
use lowerer::{lower_program, validate_program};
use optimizer::ConstantFolder;
use parser::{program_parser, token_stream};
use typechecker::TypeChecker;

#[derive(ClapParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file: String,

    #[arg(short, long)]
    config: Option<String>,

    #[arg(short, long, default_value = "out.c")]
    out: String,
}

fn main() {
    let args = Args::parse();
    let lexer = Lexer::new();

    let config_path = args
        .config
        .unwrap_or_else(|| "compiler_config.toml".to_string());
    let config = CompilerConfig::load(&config_path).unwrap_or_else(|err| {
        eprintln!(
            "Failed to load configuration file '{}': {}",
            config_path, err
        );
        std::process::exit(1);
    });

    let input = std::fs::read_to_string(&args.file).unwrap_or_else(|err| {
        eprintln!("Failed to read source file '{}': {}", args.file, err);
        std::process::exit(1);
    });
    let source_file = SourceFile::new(&args.file, &input);

    match lexer.tokenize_spanned(&input) {
        Ok(tokens) => {
            let result = program_parser().parse(token_stream(&tokens, input.len()));
            match result.into_result() {
                Ok(mut program) => {
                    let mut type_checker = TypeChecker::new();
                    match type_checker.check_program(&mut program) {
                        Ok(()) => {
                            let mut optimizer = ConstantFolder::new();
                            optimizer.fold_program(&mut program);

                            match lower_program(&program) {
                                Ok(task_ir) => match validate_program(&task_ir) {
                                    Ok(()) => {
                                        let c_code = codegen::generate_c_code(&task_ir, &config);
                                        if let Err(e) = std::fs::write(&args.out, c_code) {
                                            eprintln!(
                                                "Failed to write output file '{}': {}",
                                                args.out, e
                                            );
                                            std::process::exit(1);
                                        } else {
                                            println!(
                                                "Successfully wrote generated C code to {}",
                                                args.out
                                            );
                                        }
                                    }
                                    Err(err) => {
                                        eprintln!(
                                            "{}",
                                            source_file.format_diagnostic(
                                                "ir validation error",
                                                &err.to_string(),
                                                err.source_span(),
                                                None,
                                            )
                                        );
                                        std::process::exit(1);
                                    }
                                },
                                Err(err) => {
                                    eprintln!(
                                        "{}",
                                        source_file.format_diagnostic(
                                            "lowering error",
                                            &err.to_string(),
                                            err.source_span(),
                                            None,
                                        )
                                    );
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(errors) => {
                            for err in errors {
                                eprintln!(
                                    "{}",
                                    source_file.format_diagnostic(
                                        "type error",
                                        &err.message,
                                        err.span,
                                        None,
                                    )
                                );
                            }
                            std::process::exit(1);
                        }
                    }
                }
                Err(errs) => {
                    for err in errs {
                        let message = concise_parse_error_message(&err);
                        eprintln!(
                            "{}",
                            source_file.format_diagnostic(
                                "parse error",
                                &message,
                                Some((*err.span()).into()),
                                None,
                            )
                        );
                    }
                    std::process::exit(1);
                }
            }
        }
        Err(errors) => {
            for err in errors {
                eprintln!(
                    "{}",
                    source_file.format_diagnostic("lex error", &err.message, Some(err.span), None,)
                );
            }
            std::process::exit(1);
        }
    }
}
