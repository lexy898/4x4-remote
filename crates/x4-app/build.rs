//! Makes Cargo rebuild the crate when a message file changes: `fl!` checks message ids and
//! arguments against `i18n/en/x4.ftl` at compile time but does not tell Cargo about that file.

fn main() {
    println!("cargo::rerun-if-changed=i18n");
    println!("cargo::rerun-if-changed=i18n.toml");
}
