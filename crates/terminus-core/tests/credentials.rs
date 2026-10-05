//! The shared credentials loader: the three tiers, and PRIV-02's refusal.
//!
//! Every test here points [`SHARED_DIR_ENV`] at a directory it owns. Without
//! that, a test would read the developer's real `~/.config/jcrenshaw` - and the
//! refusal test would be asserting about the mode of a real credentials file.
//!
//! The key value each test writes is distinctive, and the assertions are that
//! no substring of it appears anywhere in a rendered error or a formatted
//! secret. "Contains no substring" rather than "is not equal to" is the point:
//! a formatter that printed the first eight characters would pass an equality
//! check and still be a leak.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use terminus_core::config::{Config, CONFIG_FILE_NAME, REDACTED};
use terminus_core::credentials::{self, Error, Permissions, SHARED_DIR_ENV};

/// A value nothing else on the machine can produce by accident.
const KEY: &str = "sk-TERMINUSCREDTEST-4b71e2-do-not-log";

/// A second one, so a precedence test can say WHICH tier answered.
const OTHER_KEY: &str = "sk-TERMINUSCREDTEST-0a55d9-from-the-file";

/// The provider namespace every test uses, and the environment variable it
/// derives.
const PROVIDER: &str = "stubprovider";
const PROVIDER_ENV: &str = "STUBPROVIDER_API_KEY";

/// `std::env::set_var` is process-global and the harness is threaded.
static ENV: Mutex<()> = Mutex::new(());

/// A test's own shared config directory and terminus config directory, with
/// every environment variable the loader reads pinned for the duration.
struct Bench {
    _dir: tempfile::TempDir,
    shared: PathBuf,
    config_dir: PathBuf,
    restore: Vec<(String, Option<std::ffi::OsString>)>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

fn bench() -> Bench {
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let shared = dir.path().join("shared");
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();

    let mut bench = Bench {
        _dir: dir,
        shared,
        config_dir,
        restore: Vec::new(),
        _guard: guard,
    };
    let shared = bench.shared.clone();
    bench.set(SHARED_DIR_ENV, Some(shared.as_os_str()));
    // Cleared rather than assumed absent: a developer with this exported would
    // otherwise see the precedence tests pass for the wrong reason.
    bench.set(PROVIDER_ENV, None);
    bench
}

impl Bench {
    fn set(&mut self, name: &str, value: Option<&std::ffi::OsStr>) {
        self.restore.push((name.to_owned(), std::env::var_os(name)));
        match value {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    fn credentials_path(&self) -> PathBuf {
        self.shared.join(credentials::CREDENTIALS_FILE_NAME)
    }

    /// Write the shared file with one section for [`PROVIDER`], at `mode`.
    fn write_shared(&self, key: &str, mode: u32) -> PathBuf {
        let path = self.credentials_path();
        std::fs::write(&path, format!("[{PROVIDER}]\napi_key = \"{key}\"\n")).unwrap();
        set_mode(&path, mode);
        path
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE_NAME)
    }

    /// A config naming the provider, and optionally carrying its own key.
    ///
    /// Written owner-only, because a `terminus.toml` carrying a key is refused
    /// at `0644` the way the shared file is: the tier and precedence tests
    /// below are about which tier answers, not about the mode of the file the
    /// harness happened to write.
    fn config(&self, api_key: Option<&str>) -> Config {
        self.config_at(api_key, 0o600)
    }

    /// The same, at a chosen mode.
    fn config_at(&self, api_key: Option<&str>, mode: u32) -> Config {
        let mut body = format!("[provider]\nenabled = true\nname = \"{PROVIDER}\"\n");
        if let Some(key) = api_key {
            body.push_str(&format!("api_key = \"{key}\"\n"));
        }
        self.write_config(&body, mode)
    }

    /// Write `terminus.toml` with this body at this mode, and load it.
    fn write_config(&self, body: &str, mode: u32) -> Config {
        std::fs::write(self.config_path(), body).unwrap();
        set_mode(&self.config_path(), mode);
        Config::load_from(&self.config_dir).unwrap()
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        for (name, previous) in self.restore.drain(..).rev() {
            match previous {
                Some(v) => std::env::set_var(&name, v),
                None => std::env::remove_var(&name),
            }
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// The fragments of a key that must appear in no rendered string.
fn fragments(key: &str) -> [&str; 3] {
    [key, &key[3..18], &key[18..24]]
}

fn assert_withholds(rendered: &str, key: &str, what: &str) {
    for fragment in fragments(key) {
        assert!(
            !rendered.contains(fragment),
            "{what} carries {fragment:?}: {rendered}"
        );
    }
}

// ---------------------------------------------------------------------------
// PRIV-02

/// A file anybody in the group can read is refused, and the refusal says which
/// file and what mode - which is everything the user needs to fix it and
/// nothing else.
#[test]
#[cfg(unix)]
fn a_group_readable_credentials_file_is_refused() {
    let bench = bench();
    let path = bench.write_shared(KEY, 0o644);

    let error = credentials::resolve(&bench.config(None)).expect_err("0644 is refused");

    assert_eq!(
        error,
        Error::TooOpen {
            path: path.clone(),
            mode: 0o644
        }
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(&path.display().to_string()),
        "the refusal does not name the file: {rendered}"
    );
    assert!(
        rendered.contains("644"),
        "the refusal does not name the mode: {rendered}"
    );
    assert_withholds(&rendered, KEY, "the refusal");
    assert_withholds(&format!("{error:?}"), KEY, "the debug-formatted refusal");
}

/// Every group and world bit, not only the read ones: a file somebody else can
/// write is a file somebody else can put their own key in.
#[test]
#[cfg(unix)]
fn any_group_or_world_bit_refuses_the_load() {
    let bench = bench();
    for mode in [0o640, 0o604, 0o660, 0o606, 0o610, 0o601] {
        bench.write_shared(KEY, mode);
        let error = credentials::resolve(&bench.config(None))
            .unwrap_err_or_panic(&format!("mode {mode:03o} was accepted"));
        assert!(
            matches!(error, Error::TooOpen { mode: found, .. } if found == mode),
            "mode {mode:03o} reported {error:?}"
        );
    }
    // The falsifying half: the same file at 0600 loads, so the loop above is
    // about the mode and not about the file.
    bench.write_shared(KEY, 0o600);
    assert!(credentials::resolve(&bench.config(None)).unwrap().is_some());
}

/// An owner-only file loads, and the value it loaded cannot be printed.
#[test]
fn an_owner_only_file_loads_and_renders_nothing_of_its_value() {
    let bench = bench();
    bench.write_shared(KEY, 0o600);

    let secret = credentials::resolve(&bench.config(None))
        .expect("0600 loads")
        .expect("the section names a key");

    // The falsifying half: the value really did come back, so the assertions
    // below are about formatters and not about an empty secret.
    assert_eq!(secret.expose(), KEY);
    assert_eq!(format!("{secret:?}"), REDACTED);
    assert_eq!(format!("{secret}"), REDACTED);
    assert_withholds(&format!("{secret:?}"), KEY, "the debug-formatted secret");
    assert_withholds(&format!("{secret}"), KEY, "the displayed secret");
}

/// PRIV-02 over terminus's OWN file (D-14): a `terminus.toml` carrying a key is
/// refused at `0644` on the same test as the shared file, and the refusal names
/// that file rather than sending the user off to chmod a shared one they may
/// not have.
#[test]
#[cfg(unix)]
fn a_group_readable_terminus_toml_carrying_a_key_is_refused() {
    let bench = bench();
    let wide = bench.config_at(Some(KEY), 0o644);

    let error = credentials::resolve(&wide).expect_err("a 0644 config carrying a key");

    assert_eq!(
        error,
        Error::ConfigTooOpen {
            path: bench.config_path(),
            mode: 0o644
        }
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(&bench.config_path().display().to_string()),
        "the refusal does not name the file: {rendered}"
    );
    assert!(
        rendered.contains("644"),
        "the refusal does not name the mode: {rendered}"
    );
    assert!(
        !rendered.contains("credentials file"),
        "the refusal points at the shared file instead of this one: {rendered}"
    );
    assert_withholds(&rendered, KEY, "the refusal");
    assert_withholds(&format!("{error:?}"), KEY, "the debug-formatted refusal");

    // The falsifying half: the same file at 0600 hands the key straight back,
    // so the refusal above is about the mode and not about the file.
    let tight = bench.config_at(Some(KEY), 0o600);
    assert_eq!(
        credentials::resolve(&tight).unwrap().unwrap().expose(),
        KEY,
        "a 0600 config carrying a key was not accepted"
    );
}

/// D-07: the refusal fires where tier 2 is CONSUMED, so a wide file whose key
/// is never the answer still loads.
///
/// Both halves are states a real user is in - judgment off is the default, and
/// a key exported in the shell outranks the file - and in neither of them may a
/// mode stop `terminus status` from reading its own config.
#[test]
#[cfg(unix)]
fn a_wide_terminus_toml_is_not_refused_while_its_key_is_not_the_answer() {
    let mut bench = bench();

    let off = bench.write_config(
        &format!("[provider]\nname = \"{PROVIDER}\"\napi_key = \"{KEY}\"\n"),
        0o644,
    );
    assert!(
        credentials::resolve(&off).unwrap().is_none(),
        "a wide config was refused while judgment was off"
    );

    bench.set(PROVIDER_ENV, Some(std::ffi::OsStr::new(OTHER_KEY)));
    let on = bench.config_at(Some(KEY), 0o644);
    assert_eq!(
        credentials::resolve(&on).unwrap().unwrap().expose(),
        OTHER_KEY,
        "a wide config was refused although the environment answered above it"
    );
}

// ---------------------------------------------------------------------------
// The three tiers (D-14)

/// The state of this machine: there is no shared file, and that is not an
/// error.
#[test]
fn an_absent_file_with_nothing_in_the_environment_yields_no_credential() {
    let bench = bench();
    assert_eq!(
        credentials::permissions(&bench.credentials_path()),
        Permissions::Absent
    );

    let found = credentials::resolve(&bench.config(None)).expect("an absent file is not an error");

    assert!(found.is_none());
}

/// Tier one over tier three.
#[test]
fn a_value_in_the_environment_outranks_one_in_the_file() {
    let mut bench = bench();
    bench.write_shared(OTHER_KEY, 0o600);
    bench.set(PROVIDER_ENV, Some(std::ffi::OsStr::new(KEY)));

    let secret = credentials::resolve(&bench.config(None)).unwrap().unwrap();

    assert_eq!(secret.expose(), KEY);
}

/// Tier one over tier two over tier three, in one pass, so the order is proven
/// rather than each pair being proven separately.
#[test]
fn the_environment_outranks_the_config_which_outranks_the_file() {
    const FROM_CONFIG: &str = "sk-TERMINUSCREDTEST-cfg-tier-two";

    let mut bench = bench();
    bench.write_shared(OTHER_KEY, 0o600);

    let config = bench.config(Some(FROM_CONFIG));
    assert_eq!(
        credentials::resolve(&config).unwrap().unwrap().expose(),
        FROM_CONFIG,
        "the product config did not outrank the shared file"
    );

    bench.set(PROVIDER_ENV, Some(std::ffi::OsStr::new(KEY)));
    assert_eq!(
        credentials::resolve(&config).unwrap().unwrap().expose(),
        KEY,
        "the environment did not outrank the product config"
    );
}

/// A file that is there and has no section for this provider is the same as no
/// file: another product's credentials are not this one's to read.
#[test]
fn a_file_naming_another_provider_yields_nothing_for_this_one() {
    let bench = bench();
    std::fs::write(
        bench.credentials_path(),
        format!("[somebody-else]\napi_key = \"{OTHER_KEY}\"\n"),
    )
    .unwrap();
    set_mode(&bench.credentials_path(), 0o600);

    assert!(credentials::resolve(&bench.config(None)).unwrap().is_none());
}

/// With no provider name there is no namespace, so there is nothing to look up
/// - and in particular no reason to read a file of other products' keys.
#[test]
fn a_config_without_a_provider_name_reads_no_shared_file() {
    let bench = bench();
    bench.write_shared(KEY, 0o600);

    std::fs::write(
        bench.config_dir.join(CONFIG_FILE_NAME),
        "[provider]\nenabled = true\n",
    )
    .unwrap();
    let config = Config::load_from(&bench.config_dir).unwrap();

    assert!(credentials::resolve(&config).unwrap().is_none());
}

/// A file that does not parse names its position and quotes none of itself:
/// every line of this file is a credential.
#[test]
fn a_shared_file_that_does_not_parse_quotes_none_of_itself() {
    let bench = bench();
    std::fs::write(
        bench.credentials_path(),
        format!("[{PROVIDER}]\napi_key = {KEY}\n"),
    )
    .unwrap();
    set_mode(&bench.credentials_path(), 0o600);

    let error = credentials::resolve(&bench.config(None)).expect_err("an unquoted value");

    let rendered = error.to_string();
    assert_withholds(&rendered, KEY, "the parse failure");
    assert_withholds(&format!("{error:?}"), KEY, "the debug-formatted failure");
    assert!(
        rendered.contains("line 2"),
        "the failure names no position at all: {rendered}"
    );
}

// ---------------------------------------------------------------------------
// The derived variable name

#[test]
fn the_environment_variable_is_derived_from_the_provider_name() {
    assert_eq!(credentials::env_var_for("openrouter"), "OPENROUTER_API_KEY");
    assert_eq!(credentials::env_var_for(PROVIDER), PROVIDER_ENV);
    assert_eq!(
        credentials::env_var_for("open-router"),
        "OPEN_ROUTER_API_KEY",
        "a separator in the name becomes a separator in the variable"
    );
    assert_eq!(
        credentials::env_var_for("a b.c"),
        "A_B_C_API_KEY",
        "nothing a config can write ends up spelling something exotic"
    );
}

/// A helper the standard library does not have: `unwrap_err` with a message.
trait UnwrapErrOr<T, E> {
    fn unwrap_err_or_panic(self, message: &str) -> E;
}

impl<T: std::fmt::Debug, E> UnwrapErrOr<T, E> for Result<T, E> {
    fn unwrap_err_or_panic(self, message: &str) -> E {
        match self {
            Ok(value) => panic!("{message}: {value:?}"),
            Err(e) => e,
        }
    }
}

// ---------------------------------------------------------------------------
// Judgment off means nothing is read (OBS-02)

/// With `enabled` unset, no tier is consulted at all: not the environment, not
/// the product config, and not the shared file.
#[test]
fn a_provider_that_is_not_enabled_resolves_no_credential() {
    let mut bench = bench();
    bench.write_shared(OTHER_KEY, 0o600);
    bench.set(PROVIDER_ENV, Some(std::ffi::OsStr::new(KEY)));

    std::fs::write(
        bench.config_dir.join(CONFIG_FILE_NAME),
        format!("[provider]\nname = \"{PROVIDER}\"\napi_key = \"{KEY}\"\n"),
    )
    .unwrap();
    let config = Config::load_from(&bench.config_dir).unwrap();

    // The falsifying half: with `enabled = true` and nothing else changed, all
    // three tiers are there and the top one answers.
    assert!(credentials::resolve(&bench.config(None)).unwrap().is_some());

    assert!(
        credentials::resolve(&config).unwrap().is_none(),
        "a credential was read while judgment was off"
    );
}

/// A file this loader never reads is a file it has nothing to refuse. Doctor
/// still reports the mode, because that is a report and not a load.
#[test]
#[cfg(unix)]
fn a_group_readable_file_is_not_even_looked_at_while_judgment_is_off() {
    let bench = bench();
    let path = bench.write_shared(KEY, 0o644);

    std::fs::write(
        bench.config_dir.join(CONFIG_FILE_NAME),
        format!("[provider]\nname = \"{PROVIDER}\"\n"),
    )
    .unwrap();
    let config = Config::load_from(&bench.config_dir).unwrap();

    assert!(credentials::resolve(&config).unwrap().is_none());
    assert_eq!(
        credentials::permissions(&path),
        Permissions::TooOpen { mode: 0o644 },
        "the mode is still reportable, which is what doctor prints"
    );
}
