//! Prints the native report of the scenarios as one line of JSON, for `cargo xtask wasm-hash`.

fn main() {
    println!("{}", gamecraft_wasmcheck::report_json());
}
