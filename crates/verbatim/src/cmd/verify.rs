//! `verbatim verify`: walk every blob against its recorded checksum (STOR-03).

use verbatim_core::{verify, Store};

use super::Failure;

pub fn run() -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    let store = Store::open(&data_dir)?;
    let report = verify::verify(&store)?;

    // Failing session keys are the data, so they go to stdout. The count is
    // commentary and goes to stderr, which also keeps stdout free of anything
    // AC3's "and no other session" clause would have to exempt.
    print!("{}", report.render());
    eprintln!(
        "{} session(s) checked, {} failed",
        report.checked,
        report.failures.len()
    );

    if report.is_ok() {
        Ok(())
    } else {
        Err(Failure::Silent)
    }
}
