//! `verbatim reindex`: rebuild every derived table from the blobs (STOR-04).

use verbatim_core::reindex;

use super::Failure;

pub fn run() -> Result<(), Failure> {
    let data_dir = super::data_dir()?;
    // `open_up_to_date` first, so a store older than this build is brought
    // forward by the same code path rather than by a second one here. On an
    // up-to-date store it is a plain open and the rebuild below is the work.
    let mut store = reindex::open_up_to_date(&data_dir)?;
    let rebuilt = reindex::reindex(&mut store)?;

    // Nothing on stdout: this command produces no data, and a phase-3 `--json`
    // caller must not have to filter a progress line out of it.
    eprintln!(
        "rebuilt {} turn(s) across {} session(s)",
        rebuilt.turns, rebuilt.sessions
    );
    Ok(())
}
