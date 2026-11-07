//! Генерация типов TL из `schema/kosta.tl` во время сборки.

use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let schema_path = Path::new(&manifest_dir).join("../schema/kosta.tl");
    println!("cargo:rerun-if-changed={}", schema_path.display());

    let source = std::fs::read_to_string(&schema_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", schema_path.display()));
    let schema = kosta_tl_gen::parse_schema(&source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", schema_path.display()));
    let code = kosta_tl_gen::generate(&schema);

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let out_path = Path::new(&out_dir).join("tl_generated.rs");
    std::fs::write(&out_path, code)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", out_path.display()));
}
