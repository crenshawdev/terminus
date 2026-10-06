//! `terminus verify`: walk every blob against its recorded checksum (STOR-03).

use serde_json::json;
use terminus_core::{verify, Store};

use super::json::Document;
use super::Failure;

pub fn run(json: bool) -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    let store = Store::open(&data_dir)?;
    let report = verify::verify(&store)?;

    if json {
        // The ids and the count in one document. Split across two streams they
        // are the same two facts, and a `--json` caller reading only stdout
        // would have to infer "how many were checked" from "how many failed".
        let mut document = Document::new("verify")
            .field("checked", report.checked as i64)
            .field(
                "failures",
                report
                    .failures
                    .iter()
                    .map(|failure| {
                        json!({"session_key": failure.session_key, "detail": failure.detail})
                    })
                    .collect::<Vec<_>>(),
            );
        if !report.is_ok() {
            document = document.failed().because(format!(
                "{} of {} session(s) failed their checksum",
                report.failures.len(),
                report.checked
            ));
        }
        document.emit();
    } else {
        // Failing session keys are the data, so they go to stdout. The count is
        // commentary and goes to stderr, which also keeps stdout free of anything
        // AC3's "and no other session" clause would have to exempt.
        print!("{}", report.render());
        // Not printed in JSON mode, and that is the rule rather than an
        // omission: `--json` means the document IS the answer, so routine
        // commentary moves into it and stderr is left for warnings. Printing
        // both would give a caller two accounts of one walk that can disagree.
        eprintln!(
            "{} session(s) checked, {} failed",
            report.checked,
            report.failures.len()
        );
    }

    if report.is_ok() {
        Ok(())
    } else {
        // Exit 1 with the document already written. A caller that parses `ok`
        // and a caller that checks the exit code must not disagree about
        // whether the store is whole.
        Err(Failure::Silent)
    }
}
