//! CLI для генератора TL: `kosta-tl-gen <schema.tl> [out.rs]`.
//!
//! Без второго аргумента печатает сгенерированный код в stdout — удобно для
//! отладки схемы (`cargo run -p kosta-tl-gen -- schema/kosta.tl`).

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(schema_path) = args.next() else {
        eprintln!("usage: kosta-tl-gen <schema.tl> [out.rs]");
        return ExitCode::FAILURE;
    };

    let source = match std::fs::read_to_string(&schema_path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("failed to read {schema_path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let schema = match kosta_tl_gen::parse_schema(&source) {
        Ok(schema) => schema,
        Err(error) => {
            eprintln!("failed to parse {schema_path}: {error}");
            return ExitCode::FAILURE;
        }
    };

    let code = kosta_tl_gen::generate(&schema);

    match args.next() {
        Some(output_path) => {
            if let Err(error) = std::fs::write(&output_path, code) {
                eprintln!("failed to write {output_path}: {error}");
                return ExitCode::FAILURE;
            }
            println!(
                "generated {} constructors -> {}",
                schema.constructors.len(),
                output_path
            );
        }
        None => print!("{code}"),
    }
    ExitCode::SUCCESS
}
