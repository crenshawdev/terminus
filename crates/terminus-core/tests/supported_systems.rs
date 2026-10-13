//! Terminus refuses to build anywhere but Linux and macOS.
//!
//! A build for another system needs that system's toolchain, so the refusal is
//! proved here by reading it from core's `lib.rs`. Not gated on `testkit`, so
//! `cargo test -p terminus-core` always runs it.

const GUARD: &str = r#"#[cfg(not(any(target_os = "linux", target_os = "macos")))]"#;
const REFUSAL: &str = r#"compile_error!("Terminus supports Linux and macOS only.");"#;

#[test]
fn a_build_for_any_other_system_stops_at_compile_time() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs");
    let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let lines: Vec<&str> = source.lines().map(str::trim).collect();

    let guard = lines.iter().position(|l| *l == GUARD);
    let refusal = lines.iter().position(|l| *l == REFUSAL);
    let first_mod = lines.iter().position(|l| l.starts_with("pub mod "));

    let guard = guard.unwrap_or_else(|| panic!("lib.rs has no line `{GUARD}`"));
    let refusal = refusal.unwrap_or_else(|| panic!("lib.rs has no line `{REFUSAL}`"));
    let first_mod = first_mod.expect("lib.rs has no `pub mod` line");
    assert_eq!(
        refusal,
        guard + 1,
        "the compile_error! line does not directly follow its guard"
    );
    assert!(
        refusal < first_mod,
        "the refusal comes after the first `pub mod`"
    );
}
