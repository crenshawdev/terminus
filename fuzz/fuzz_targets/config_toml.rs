//! `terminus.toml`, loaded the way every command loads it.
#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;
use terminus_core::Config;

fuzz_target!(|data: &[u8]| {
    let dir = terminus_fuzz::scratch("config-under-test");
    std::fs::write(dir.join("terminus.toml"), data).expect("write terminus.toml");
    let Ok(config) = Config::load_from(&dir) else {
        return;
    };
    let _ = config.transcript_roots();
    let _ = config.excludes_everything();
    let _ = config.retention_selects_nothing();
    for path in ["/", "/home/user/project", "/home/user/project/sub", "relative/path"] {
        let _ = config.excludes_path(Path::new(path));
        let _ = config.retention_for(Some(path));
    }
    let _ = config.retention_for(None);
    let _ = (config.brief_chars(), config.prompt_chars(), config.capture_mode());
    let _ = (config.snapshots_kept(), config.snapshot_interval_hours());
});
