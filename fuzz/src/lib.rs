//! What the fuzz targets share: scratch directories of their own, so no target
//! reads or writes the real archive or the real `terminus.toml`.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::OnceLock;

use terminus_core::{Config, Store};

/// A data directory holding an empty store and an empty config directory,
/// made once per process. `TERMINUS_DATA_DIR` and `TERMINUS_CONFIG_DIR` point
/// at them, so code that resolves its own directories lands here.
pub fn sandbox() -> &'static Sandbox {
    static SANDBOX: OnceLock<Sandbox> = OnceLock::new();
    SANDBOX.get_or_init(|| {
        let data_dir = scratch("data");
        let config_dir = scratch("config");
        Store::open(&data_dir).expect("create the fuzz store");
        std::env::set_var("TERMINUS_DATA_DIR", &data_dir);
        std::env::set_var("TERMINUS_CONFIG_DIR", &config_dir);
        let config = Config::load_from(&config_dir).expect("load the empty fuzz config");
        Sandbox {
            data_dir,
            config,
        }
    })
}

pub struct Sandbox {
    pub data_dir: PathBuf,
    pub config: Config,
}

impl Sandbox {
    /// Remove what one injection wrote, so a long run does not fill the disk
    /// with decision records.
    pub fn reset(&self) {
        for name in ["decisions", "injection"] {
            let _ = std::fs::remove_dir_all(self.data_dir.join(name));
        }
    }
}

/// A config with `[privacy] redact_recall` on, loaded once per process.
pub fn redacting() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = scratch("redacting");
        std::fs::write(dir.join("terminus.toml"), "[privacy]\nredact_recall = true\n")
            .expect("write the redacting config");
        Config::load_from(&dir).expect("load the redacting config")
    })
}

/// `<name>` inside this process's own private temporary directory, created if
/// it is not there. The parent has a random name and mode 0700, so no other
/// user can claim or read it first.
pub fn scratch(name: &str) -> PathBuf {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    let root = ROOT.get_or_init(|| {
        tempfile::Builder::new()
            .prefix("terminus-fuzz-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .expect("create the fuzz scratch directory")
    });
    let dir = root.path().join(name);
    std::fs::create_dir_all(&dir).expect("create a fuzz scratch directory");
    dir
}

/// Split fuzz input into the pieces a target needs, at the first `\n`.
pub fn split_line(data: &[u8]) -> (&[u8], &[u8]) {
    match data.iter().position(|b| *b == b'\n') {
        Some(at) => (&data[..at], &data[at + 1..]),
        None => (data, &[]),
    }
}
