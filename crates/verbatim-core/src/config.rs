//! Verbatim's own config: which transcript trees to walk, and which projects
//! never to look at.
//!
//! The file is `verbatim.toml` inside verbatim's config directory, and
//! `config.toml` in that same directory is deliberately never read (D-15). The
//! legacy tool wrote one - 184 bytes, `base_dir = "/data/verbatim"` - and
//! adopting it would point the new store at the legacy data directory whose
//! import `.planning/PROJECT.md` defers.
//!
//! A missing config file is not an error. It yields the defaults, which is the
//! state every user starts in.
//!
//! # Exclusion has two entry points, because its two callers know two different
//! things
//!
//! [`Config::excludes_encoded_dir`] answers **before any file is opened**. All
//! it has is the encoded project directory name, because `cwd` only exists on a
//! parsed record and reading a record means opening the file - which is exactly
//! what ING-08 forbids for an excluded project (D-22). It is therefore an exact
//! match on the encoded name, plus one fixed-literal worktree clause, and not a
//! prefix match: the encoding is provably lossy (D-07), so an
//! extension-tolerant rule cannot tell a child directory from a hyphenated
//! sibling and `-data-projects-cadence` would silently swallow
//! `-data-projects-cadence-research`.
//!
//! [`Config::excludes_path`] answers wherever a real filesystem path is
//! available, and there it is an ordinary subtree match on path components,
//! which is unambiguous. That is the read-side predicate (D-23).
//!
//! Both read the same configured strings, so those strings are normalized once
//! ([`normalize`]) before either sees them. The component test is indifferent
//! to a trailing separator and the encoded test is not, and two predicates that
//! disagree about which projects are excluded is precisely the read-then-filter
//! ING-08 forbids.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Verbatim's config file. `config.toml` beside it is the legacy tool's and is
/// never read (D-15).
pub const CONFIG_FILE_NAME: &str = "verbatim.toml";

/// Points verbatim's config directory somewhere else, the way
/// `VERBATIM_DATA_DIR` does for the store.
pub const CONFIG_DIR_ENV: &str = "VERBATIM_CONFIG_DIR";

/// Claude Code's own override for its config directory. Read as a **single**
/// directory (D-15): the design brief says roots accept a list, but the
/// separator convention there is Claude Code's and the variable is unset on the
/// development machine, so multi-root is supported through `verbatim.toml`
/// rather than by guessing somebody else's separator.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";

/// The subdirectory of a Claude config directory that holds the transcripts.
pub const PROJECTS_SUBDIR: &str = "projects";

/// The default Claude config directory, relative to the user's home.
pub const DEFAULT_CLAUDE_DIR: &str = ".claude";

/// What a path separator encodes to under D-07's rule.
///
/// [`encode`] maps every non-alphanumeric character to this, so a subdirectory
/// of an encoded path is that path, this, and the rest - and so is a sibling
/// whose name merely contains a `-` or a `.`. Telling those two apart is what
/// [`descends_to`] is for.
const ENCODED_SEPARATOR: char = '-';

/// How many directory entries [`descends_to`] may examine before it gives up
/// and lets the caller fail safe.
///
/// The search is pruned to the branches whose encoding is still a prefix of the
/// candidate, so a real tree costs a handful of entries; this only bounds a
/// pathological one. Exhausting it is indistinguishable from an unreadable
/// directory on purpose - both mean "could not resolve", and both exclude.
const DIR_SCAN_BUDGET: usize = 4_096;

/// What `verbatim.toml` may contain.
///
/// Unknown keys are ignored rather than rejected: this file will grow across
/// phases, and a store written by a newer binary must not make an older one
/// refuse to start.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileConfig {
    /// Claude config directories, each of whose `projects` subdirectory is a
    /// tree to walk. An explicit list wins over [`CLAUDE_CONFIG_DIR_ENV`].
    #[serde(default)]
    roots: Vec<String>,
    /// Project paths - real filesystem paths, as the user writes them - that
    /// are never read and never returned by a read path.
    #[serde(default)]
    exclude: Vec<String>,
    /// How much context injection may write, per event (INJ-01, D-18).
    #[serde(default)]
    injection: FileInjection,
    /// The model provider observations may ask for judgment (OBS-05, D-10).
    #[serde(default)]
    provider: FileProvider,
    /// How much of each project's history to keep (RET-01, D-01).
    #[serde(default)]
    retention: FileRetention,
}

/// The `[retention]` table of `verbatim.toml` (RET-01, D-01).
///
/// **Read and never written.** No command in this workspace writes this file
/// and none can: the `toml` dependency is `default-features = false` with the
/// serializer half deliberately absent (root `Cargo.toml`), so "retention is
/// configured by hand-editing the file" is a property of what is linked rather
/// than a convention a later command could quietly break.
///
/// Every key is optional and a missing table means the defaults, under the same
/// rule `[injection]` and `[provider]` follow. Here the defaults are the OFF
/// state, which is RET-01 itself: a store keeps everything unless this table
/// says otherwise.
///
/// The per-project keys are project paths **as the user writes them** -
/// `[retention.project."/data/code/scratch"]` - and they are put through the
/// same [`normalize`] the exclusions go through before anything compares them
/// (D-01). The design brief's own example writes a bare `scratch` while every
/// stored project key is a canonical git toplevel, so a rule that matched by
/// string equality against `session_meta.project` would silently match nothing
/// forever and read as broken rather than as misconfigured.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileRetention {
    action: Option<String>,
    age_days: Option<i64>,
    #[serde(default)]
    project: BTreeMap<String, FileRetentionTable>,
}

/// One `[retention.project."<path>"]` table: the same two keys as the global
/// one, so a project overrides the whole rule rather than half of it.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileRetentionTable {
    action: Option<String>,
    age_days: Option<i64>,
}

/// The `[provider]` table of `verbatim.toml` (D-10).
///
/// Every key optional, an unrecognized key ignored under the same rule as one
/// at the top level, and a missing table meaning the defaults - which are "ask
/// no model anything".
///
/// `api_key` arrives as a plain `String` and is wrapped in a [`Secret`] by
/// [`Config::resolve`] rather than being deserialized straight into one. A
/// serde error names the type it wanted, and the one place a value can still
/// reach a stream from here is the TOML parser's own excerpt of a malformed
/// line - which [`withhold_secret_excerpt`] takes out.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileProvider {
    enabled: Option<bool>,
    base_url: Option<String>,
    model: Option<String>,
    name: Option<String>,
    api_key: Option<String>,
    local: Option<bool>,
    daily_token_budget: Option<u64>,
    response_format: Option<String>,
}

/// The `[injection]` table of `verbatim.toml`.
///
/// Every key is optional so that a table naming one budget leaves the other at
/// its default, and an unrecognized key inside it is ignored under the same
/// rule as one at the top level.
#[derive(Debug, Clone, Default, Deserialize)]
struct FileInjection {
    brief_chars: Option<usize>,
    prompt_chars: Option<usize>,
}

/// The resume brief's budget when nothing configures one.
///
/// Characters, not tokens (D-16): the workspace has eight dependencies, each
/// justified in the root `Cargo.toml` against a measured 0.408 ms startup
/// floor, and none of them is a tokenizer - the existing budgets
/// ([`crate::recall::EXCERPT_CHARS`], `index::MAX_BODY_BYTES`) are already
/// byte-shaped for the same reason. 6,000 is roughly 1.5k tokens at four
/// characters a token, and it is under the 10,000-character ceiling past which
/// Claude Code 2.1.237 persists a hook's stdout to disk and replaces it with a
/// reference - a brief past that stops being context and becomes a file path.
pub const DEFAULT_BRIEF_CHARS: usize = 6_000;

/// The prompt injection's budget when nothing configures one.
///
/// Smaller than [`DEFAULT_BRIEF_CHARS`] because it is paid on every prompt
/// rather than once a session, and because INJ-03 caps it at three turns.
pub const DEFAULT_PROMPT_CHARS: usize = 4_000;

/// What a [`Secret`] renders as, everywhere, under every formatter.
///
/// A fixed marker rather than an empty string, so a reader of a message can
/// tell "there was a credential here and it was withheld" from "there was
/// nothing here" - the second would make a missing key look like a wrong one.
pub const REDACTED: &str = "[redacted]";

/// A credential value, in the one wrapper in this workspace that cannot print
/// itself.
///
/// **The point is what it does NOT have.** No `Debug` that shows the value, no
/// `Display`, no `AsRef<str>`, no `Deref`, no `Into<String>`. The only way to
/// the bytes is [`Secret::expose`], which is spelled to be conspicuous at the
/// call site and is called in exactly the places that build a request. Every
/// other route - a formatted config, a `{}` in an error message, a `{:?}` in a
/// panic - renders [`REDACTED`].
///
/// `Config` derives `Debug` and is formatted whole in more than one place; this
/// type is what makes that derive safe (PRIV-01).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a credential value.
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    /// The value itself, for the one caller that has to put it on the wire.
    ///
    /// Named `expose` and not `as_str` on purpose: a reviewer grepping for
    /// where a credential leaves its wrapper finds every site in one search,
    /// and a call added in the wrong place is visible in a diff.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Is the wrapped value empty?
    ///
    /// An empty string in a config or an environment variable means "unset"
    /// wherever it appears in this workspace ([`non_empty_var`]), and the
    /// caller cannot ask by looking.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Which structured-output mode the endpoint is asked for (D-09).
///
/// Not a second request SHAPE and not a branch on who the provider is - it is
/// one field of one body, set by one key, and every mode goes down the same
/// code path. What forced it into the config was an endpoint that cannot do the
/// default: `deepseek-chat` answers 400 `This response_format type is
/// unavailable now` to `json_schema` however well-formed it is, while accepting
/// `json_object`. D-09 named tool-calling as the fallback and deferred it as
/// speculative; this is the non-speculative half, measured 2026-08-22.
///
/// The schema reaches the model either way: it is written into the instruction
/// turn (`crate::observe::judgment`), and every claim's `turn_id` is validated
/// against `turns` before a row is stored. What the stricter modes add is
/// enforcement at the endpoint, not the anchoring guarantee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResponseFormat {
    /// `{"type":"json_schema","json_schema":{name,strict,schema}}` - the
    /// endpoint holds the answer to the schema. The default, and what OpenAI,
    /// OpenRouter and vLLM implement.
    #[default]
    JsonSchema,
    /// `{"type":"json_object"}` - the endpoint guarantees parseable JSON and
    /// nothing about its shape. DeepSeek's only structured mode.
    JsonObject,
    /// No `response_format` field at all, for an endpoint that rejects the key
    /// itself. The instruction turn is then the only thing asking for JSON.
    None,
}

impl ResponseFormat {
    /// Parse the config value. An unrecognized one is the default, under the
    /// same rule that ignores an unrecognized KEY: this file grows across
    /// phases and a typo must not stop a store from opening.
    fn parse(value: &str) -> ResponseFormat {
        match value.trim().to_ascii_lowercase().as_str() {
            "json_object" => ResponseFormat::JsonObject,
            "none" => ResponseFormat::None,
            _ => ResponseFormat::JsonSchema,
        }
    }
}

/// The resolved `[provider]` block (OBS-05, D-05, D-10).
///
/// Base URL, model and key are what differ between a local ollama and a remote
/// OpenAI-compatible endpoint; there is no second request shape behind any of
/// these fields. `response_format` is the one exception AC4 forced, and it
/// selects a field's value rather than a code path.
///
/// Nothing here is parsed as a URL, resolved as a host or looked up in DNS.
/// There is no `url` crate in this workspace on purpose (D-13): a name
/// resolution is itself a connection PRIV-03 bars, and it would happen while
/// merely reading a config file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Provider {
    enabled: bool,
    base_url: Option<String>,
    model: Option<String>,
    name: Option<String>,
    api_key: Option<Secret>,
    local: bool,
    daily_token_budget: Option<u64>,
    response_format: ResponseFormat,
}

/// What retention does to a session that has aged past its policy (RET-01).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RetentionAction {
    /// Keep it. The default, the whole product's posture, and what an
    /// unrecognized `action` resolves to.
    #[default]
    Keep,
    /// Empty the blob and mark the session evicted, leaving the row, its
    /// metadata and every derived row in place (RET-02).
    Evict,
    /// Remove the session entirely - and only once Claude Code's own
    /// `cleanupPeriodDays` has already removed its transcript (D-02).
    Delete,
}

impl RetentionAction {
    /// Parse the config value. An unrecognized one is [`RetentionAction::Keep`],
    /// under the same rule [`ResponseFormat::parse`] states for an unrecognized
    /// value and for the same reason plus a sharper one: this file grows across
    /// phases, and a typo here must not make a store start deleting.
    fn parse(value: &str) -> RetentionAction {
        match value.trim().to_ascii_lowercase().as_str() {
            "evict" => RetentionAction::Evict,
            "delete" => RetentionAction::Delete,
            _ => RetentionAction::Keep,
        }
    }
}

/// One resolved retention rule: what to do, and how old a session has to be
/// before it is done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub action: RetentionAction,
    /// How many days a session's last turn must predate before the action
    /// applies. Zero is off, and it is what an absent, zero or negative
    /// `age_days` all resolve to.
    pub age_days: u32,
}

impl RetentionPolicy {
    /// Can this policy ever name a session?
    ///
    /// "Off by default" is a property of the resolved VALUE rather than of a
    /// caller remembering to check two fields (RET-01). Both halves have to be
    /// set for anything to happen: an `action` with no age names every session
    /// that ever existed, and an age with no action names them for no purpose.
    pub fn selects_nothing(&self) -> bool {
        self.age_days == 0 || self.action == RetentionAction::Keep
    }
}

/// The resolved `[retention]` block (RET-01, D-01).
///
/// The project keys are [`normalize`]d, exactly as the exclusions are, and a
/// key that normalizes to nothing is dropped exactly as an exclusion is - an
/// empty string here would be an ancestor of every path there is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Retention {
    global: RetentionPolicy,
    projects: Vec<(String, RetentionPolicy)>,
}

/// The resolved config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    roots: Vec<PathBuf>,
    exclusions: Vec<String>,
    /// D-07's encoding of each exclusion, case-folded where the platform is,
    /// computed once so the pre-open test costs a comparison per project
    /// directory rather than a re-encode.
    encoded_exclusions: Vec<String>,
    brief_chars: usize,
    prompt_chars: usize,
    provider: Provider,
    retention: Retention,
}

/// The defaults, spelled once. Derived `Default` would give both budgets zero,
/// which is a config that silently injects nothing - and `Config::default()` is
/// what several callers build when they have no file to read.
impl Default for Config {
    fn default() -> Config {
        Config {
            roots: Vec::new(),
            exclusions: Vec::new(),
            encoded_exclusions: Vec::new(),
            brief_chars: DEFAULT_BRIEF_CHARS,
            prompt_chars: DEFAULT_PROMPT_CHARS,
            // Every field false, absent or zero, and that is the right default
            // here where it is the wrong one for the budgets: judgment is
            // opt-in and off (OBS-02), and `local` absent means remote and
            // therefore filtered (D-13).
            provider: Provider::default(),
            // Every field at its zero, and here that IS the specified state:
            // `Keep` with no age selects nothing, so a user who never wrote a
            // `[retention]` table has retention off (RET-01).
            retention: Retention::default(),
        }
    }
}

impl Config {
    /// Load from verbatim's config directory.
    pub fn load() -> Result<Config> {
        let dir = config_dir()?;
        Config::load_from(&dir)
    }

    /// Load from a named config directory.
    ///
    /// Only `verbatim.toml` inside it is read. A `config.toml` sitting beside
    /// it is the legacy tool's file and is not this program's (D-15).
    pub fn load_from(config_dir: &Path) -> Result<Config> {
        let path = config_dir.join(CONFIG_FILE_NAME);
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<FileConfig>(&text).map_err(|source| {
                // The file exists and does not parse, so the user meant
                // something by it: naming the file and the position beats
                // falling back to defaults and walking the wrong tree.
                Error::ConfigParse {
                    path: path.clone(),
                    detail: withhold_secret_excerpt(&source.to_string()),
                }
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        Config::resolve(file)
    }

    /// A config built in memory, for callers that have no file. Test support
    /// and nothing else uses this today.
    ///
    /// The signature is the one every existing caller passes, and the injection
    /// budgets it does not name come out at their defaults: a new argument here
    /// would be an edit to every test in the workspace that builds a config.
    pub fn from_parts(roots: Vec<PathBuf>, exclusions: Vec<String>) -> Config {
        let exclusions: Vec<String> = exclusions.iter().filter_map(|e| normalize(e)).collect();
        let encoded_exclusions = exclusions.iter().map(|e| fold(&encode(e))).collect();
        Config {
            roots,
            exclusions,
            encoded_exclusions,
            ..Config::default()
        }
    }

    fn resolve(file: FileConfig) -> Result<Config> {
        let roots = if !file.roots.is_empty() {
            file.roots.iter().map(PathBuf::from).collect()
        } else if let Some(dir) = non_empty_var(CLAUDE_CONFIG_DIR_ENV) {
            // Replaces the DEFAULT only. An explicit `roots` list is verbatim's
            // own configuration and outranks Claude Code's environment.
            vec![PathBuf::from(dir)]
        } else {
            vec![home_dir()?.join(DEFAULT_CLAUDE_DIR)]
        };
        let mut config = Config::from_parts(roots, file.exclude);
        if let Some(chars) = file.injection.brief_chars {
            config.brief_chars = chars;
        }
        if let Some(chars) = file.injection.prompt_chars {
            config.prompt_chars = chars;
        }
        config.provider = Provider {
            enabled: file.provider.enabled.unwrap_or(false),
            base_url: non_empty(file.provider.base_url),
            model: non_empty(file.provider.model),
            name: non_empty(file.provider.name),
            api_key: non_empty(file.provider.api_key).map(Secret::new),
            local: file.provider.local.unwrap_or(false),
            daily_token_budget: file.provider.daily_token_budget,
            response_format: non_empty(file.provider.response_format)
                .as_deref()
                .map(ResponseFormat::parse)
                .unwrap_or_default(),
        };
        config.retention = resolve_retention(file.retention);
        Ok(config)
    }

    /// May observations ask a model for judgment (OBS-02)?
    ///
    /// False unless the config says otherwise, including for a config file that
    /// has no `[provider]` table at all. Nothing in this workspace resolves a
    /// credential or builds a request while this is false.
    pub fn provider_enabled(&self) -> bool {
        self.provider.enabled
    }

    /// The endpoint's base, which `chat/completions` is appended to (D-05).
    ///
    /// Handed back exactly as written. It is not parsed, not validated and not
    /// resolved (D-13): there is no `url` crate here, and a DNS lookup is a
    /// connection PRIV-03 bars.
    pub fn provider_base_url(&self) -> Option<&str> {
        self.provider.base_url.as_deref()
    }

    /// The model name to put in the request body.
    pub fn provider_model(&self) -> Option<&str> {
        self.provider.model.as_deref()
    }

    /// The provider namespace all three credential tiers key off (D-14).
    ///
    /// It names the section of the shared credentials file and the spelling of
    /// the environment variable; `crate::credentials` documents both. With no
    /// name there is no namespace to look a credential up under, so only the
    /// `api_key` written in this file can be found.
    pub fn provider_name(&self) -> Option<&str> {
        self.provider.name.as_deref()
    }

    /// The product-config tier of D-14's precedence: a value, not a reference.
    ///
    /// Behind [`Secret`], so a `{:?}` of the whole config cannot render it.
    pub fn provider_api_key(&self) -> Option<&Secret> {
        self.provider.api_key.as_ref()
    }

    /// Is the configured provider on this machine (D-13)?
    ///
    /// **This key describes the DESTINATION, not the address.** It is declared
    /// and never inferred: nothing here looks at the base URL to decide, because
    /// deciding would mean parsing a host and resolving a name, and a name
    /// resolution is itself a connection PRIV-03 bars.
    ///
    /// Absent or false means remote, and therefore filtered, so a user who
    /// forgets the key pays the egress filter unnecessarily - which is
    /// harmless. The failure that is not harmless is the other direction: set
    /// `local = true` on a reverse proxy that forwards offsite and unfiltered
    /// session text goes offsite. The address being `127.0.0.1` is not the
    /// question; where the bytes end up is.
    pub fn provider_local(&self) -> bool {
        self.provider.local
    }

    /// How many tokens a day the provider may be paid for (OBS-06, D-11).
    ///
    /// The one cost control that is a config key, because it is the one facing
    /// money. The minimum turn count and the truncation budget are quality
    /// knobs and stay compile-time constants. `None` is no cap - which is the
    /// safe default only because judgment is off unless
    /// [`Config::provider_enabled`] says otherwise.
    pub fn provider_daily_token_budget(&self) -> Option<u64> {
        self.provider.daily_token_budget
    }

    /// Which structured-output mode to ask the endpoint for (D-09).
    ///
    /// [`ResponseFormat::JsonSchema`] unless the config says otherwise, so an
    /// endpoint that implements the default needs no key and AC4's "only base
    /// URL, model and key" holds for every provider that does. A provider that
    /// does not - DeepSeek - is the reason the key exists.
    pub fn provider_response_format(&self) -> ResponseFormat {
        self.provider.response_format
    }

    /// How many characters the SessionStart resume brief may carry (INJ-01).
    pub fn brief_chars(&self) -> usize {
        self.brief_chars
    }

    /// How many characters one UserPromptSubmit injection may carry (INJ-03).
    pub fn prompt_chars(&self) -> usize {
        self.prompt_chars
    }

    /// The retention policy in force for a session stored under this project
    /// key (RET-01, D-01).
    ///
    /// **The deepest configured key that COVERS the session's project wins**,
    /// through [`covers`] - the same component-wise ancestor test
    /// [`Config::excludes_path`] and `recall::scope` share, and the same
    /// deepest-wins rule `recall::scope` already applies to project keys. So
    /// `[retention.project."/data/code/scratch"]` governs a session archived
    /// under `/data/code/scratch/sub` and does not govern one under
    /// `/data/code/scratch-other`, and where both `/data/code` and
    /// `/data/code/scratch` are configured the more specific one decides.
    ///
    /// A session no configured key covers - including one carrying no project
    /// key at all, which one real transcript per 3,416 does - gets the global
    /// table. That is the widest rule and it is still off unless the global
    /// table says otherwise.
    pub fn retention_for(&self, project: Option<&str>) -> RetentionPolicy {
        let Some(project) = project else {
            return self.retention.global;
        };
        let project = Path::new(project);
        let mut best: Option<(usize, RetentionPolicy)> = None;
        for (key, policy) in &self.retention.projects {
            let Some(depth) = covers(Path::new(key), project) else {
                continue;
            };
            if best.is_none_or(|(deepest, _)| depth > deepest) {
                best = Some((depth, *policy));
            }
        }
        best.map_or(self.retention.global, |(_, policy)| policy)
    }

    /// Is there any session, anywhere, this config's retention could name?
    ///
    /// Asked once so the retention step can issue no query at all in the state
    /// every user starts in. It is deliberately answered from the config alone
    /// and never from the store: "retention is off" must be a fact about what
    /// was configured, not about what happens to be archived today.
    pub fn retention_selects_nothing(&self) -> bool {
        self.retention.global.selects_nothing()
            && self
                .retention
                .projects
                .iter()
                .all(|(_, policy)| policy.selects_nothing())
    }

    /// The Claude config directories, in the order they were configured.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// The trees a pass walks: each root's `projects` subdirectory.
    pub fn transcript_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.join(PROJECTS_SUBDIR)).collect()
    }

    /// The excluded project paths, exactly as configured.
    pub fn exclusions(&self) -> &[String] {
        &self.exclusions
    }

    /// Does the config exclude every project there can be?
    ///
    /// True only for an exclusion that is the filesystem root, which is the one
    /// the encoded test cannot express on its own: root encodes to a bare
    /// separator, and `-data` extends `-` with no second separator between them
    /// because for root the separator IS the encoding. [`Config::excludes_path`]
    /// already reads `/` as the whole tree, so without this the two predicates
    /// would disagree on the single exclusion that means "read nothing".
    pub fn excludes_everything(&self) -> bool {
        self.exclusions.iter().any(|e| is_filesystem_root(e))
    }

    /// The pre-open test (D-09, D-22): is this encoded project directory name
    /// excluded?
    ///
    /// Exact equality against the encoded exclusion, or a name that extends it
    /// past an encoded separator and is confirmed against the filesystem to be
    /// a real subdirectory of the excluded path.
    ///
    /// The extension case is the ambiguity D-07 proved: `-` encodes the
    /// separator and also a literal `-` or `.`, so `-data-code-foo-bar` is
    /// `/data/code/foo/bar` and `/data/code/foo-bar` and
    /// `/data/code/foo.bar` at once. D-09 asked for a segment-boundary match
    /// and for `-data-projects-cadence-research` not to match
    /// `-data-projects-cadence`, which are not both satisfiable in the encoded
    /// space alone - so this resolves them outside it, by asking the filesystem
    /// which of those paths exists ([`descends_to`]). That reads directory
    /// entries and opens no transcript, which is what AC5 and D-22 constrain.
    ///
    /// Unresolvable means excluded: an excluded directory that is unreadable or
    /// gone, or a tree too large to search, leaves the question open, and the
    /// safe answer for an exclusion boundary is not to read. The cost of being
    /// wrong that way is a project that goes unarchived and can be archived
    /// later; the cost of the other way is bytes the user said never to read.
    ///
    /// This decides only whether to OPEN. [`Config::excludes_path`] is the
    /// exact test, and it is what decides whether anything is hidden.
    pub fn excludes_encoded_dir(&self, dir_name: &str) -> bool {
        let name = fold(dir_name);
        self.exclusions
            .iter()
            .zip(&self.encoded_exclusions)
            .any(|(excluded, encoded)| {
                if is_filesystem_root(excluded) {
                    return true;
                }
                if name == *encoded {
                    return true;
                }
                let extends = name
                    .strip_prefix(encoded.as_str())
                    .is_some_and(|rest| rest.starts_with(ENCODED_SEPARATOR));
                if !extends {
                    return false;
                }
                descends_to(Path::new(excluded), &name).unwrap_or(true)
            })
    }

    /// The read-side test (D-23): is this real path inside an excluded project?
    ///
    /// A subtree match on path components, so `/data/projects/cadence` covers
    /// `/data/projects/cadence/sub` and does not cover
    /// `/data/projects/cadence-research`. Components rather than string
    /// prefixes, because that is what makes the separator boundary exact and
    /// what makes a trailing slash in the config irrelevant.
    pub fn excludes_path(&self, path: &Path) -> bool {
        self.exclusions
            .iter()
            .any(|excluded| covers(Path::new(excluded), path).is_some())
    }
}

/// Turn the `[retention]` table into the value every caller reads (D-01).
///
/// Two things happen here and nowhere else. Every per-project key goes through
/// [`normalize`], so a rule written `~/code/scratch/` or `/data/code/../code`
/// names the same subtree the exclusions would - and a key that normalizes to
/// nothing is dropped rather than kept as an empty string that covers every
/// path there is. And `age_days` is CLAMPED rather than rejected: deserializing
/// it into an unsigned integer instead would make a negative number fail the
/// whole config load, which is a typo in a key that only ever turns something
/// on stopping `verbatim status` from running at all. Clamping sends it to
/// zero, which is off.
fn resolve_retention(file: FileRetention) -> Retention {
    fn rule(action: Option<String>, age_days: Option<i64>) -> RetentionPolicy {
        RetentionPolicy {
            action: non_empty(action)
                .as_deref()
                .map(RetentionAction::parse)
                .unwrap_or_default(),
            age_days: age_days.unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32,
        }
    }

    Retention {
        global: rule(file.action, file.age_days),
        projects: file
            .project
            .into_iter()
            .filter_map(|(key, table)| {
                normalize(&key).map(|key| (key, rule(table.action, table.age_days)))
            })
            .collect(),
    }
}

/// One configured exclusion, reduced to the single spelling both predicates
/// agree on - or `None` when it names nothing.
///
/// The two entry points disagree about spelling unless something makes them
/// agree here. [`Config::excludes_path`] compares path *components*, so
/// `/data/code/demo/`, `/data/code//demo` and `/data/code/./demo` are all the
/// same subtree to it. [`Config::excludes_encoded_dir`] compares an *encoded
/// string*, where each of those encodes to a different name and only one of
/// them can equal a real project directory's. Left unnormalized, a trailing
/// separator is the whole read-then-filter failure ING-08 forbids: the
/// pre-open test never matches, every file in the project is opened and
/// archived, and only the read path hides it afterwards.
///
/// Walking the components rebuilds the path in the form `excludes_path`
/// already reads, which is what makes the encoding of it meaningful. `..` is
/// resolved here rather than left standing, and a leading `~` is expanded:
/// both are spellings a user writes and neither can ever match, so left alone
/// they are an exclusion that fails silently OPEN - strictly worse than the
/// trailing separator above, which at least still hid the sessions.
///
/// Resolving `..` lexically rather than through the filesystem is deliberate.
/// The excluded directory is frequently gone - the whole worktree case is
/// exactly that (D-06) - and an exclusion must not stop meaning what it says
/// because the directory it names was deleted.
///
/// An exclusion that reduces to nothing at all is dropped rather than kept as
/// an empty string, which prefixes every name there is.
fn normalize(raw: &str) -> Option<String> {
    let expanded = match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            home_dir().ok()?.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(raw),
    };

    let mut normalized = PathBuf::new();
    for component in expanded.components() {
        match component {
            // `/a/../b` is `/b`, and `/..` is `/`: popping nothing at the root
            // is what the kernel does too.
            std::path::Component::ParentDir => {
                if !normalized.pop() && normalized.as_os_str().is_empty() {
                    return None;
                }
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other),
        }
    }

    // A relative exclusion names no project. Both predicates compare against
    // absolute paths - a project directory encodes a `cwd`, which is always
    // absolute - so `code/demo` matches nothing, anywhere, forever. Dropping it
    // is not worse than keeping it and is at least one consistent answer.
    if !normalized.has_root() {
        return None;
    }

    let text = normalized.into_os_string().into_string().ok()?;
    (!text.is_empty()).then_some(text)
}

/// Is this exclusion the filesystem root - the one that means "read nothing"?
///
/// `RootDir` and nothing else. Not `Path::parent().is_none()`, which is also
/// true of a Windows prefix with no directory below it (`C:`,
/// `\\server\share`): those name a real subtree that [`Config::excludes_path`]
/// matches component-wise, so answering "everything" for them would put the two
/// predicates back into exactly the disagreement [`normalize`] exists to end.
fn is_filesystem_root(path: &str) -> bool {
    let mut components = Path::new(path).components();
    components.next() == Some(std::path::Component::RootDir) && components.next().is_none()
}

/// D-07's encoding: every character outside `[A-Za-z0-9]` becomes `-`.
///
/// Applying this to a transcript's first `cwd` reproduces its containing
/// directory name for 1,252 of 1,252 real files, so the encoding is exact - and
/// exactly lossy, which is why nothing ever runs it backwards.
pub fn encode(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Path comparison is case-insensitive on Windows and macOS and case-sensitive
/// elsewhere (`.planning/PROJECT.md` constraints).
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn fold(s: &str) -> String {
    s.to_lowercase()
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn fold(s: &str) -> String {
    s.to_owned()
}

/// Does some real directory beneath `root` encode to `name`?
///
/// `Some(true)` when one does - `name` is a subdirectory of `root` and the
/// caller excludes it. `Some(false)` only when the search was COMPLETE and
/// nothing on the way to `name` exists at all, which is what a path that merely
/// encodes the same way looks like; the caller leaves that one alone. `None`
/// whenever the question could not be answered, and the caller treats `None` as
/// excluded.
///
/// The walk descends only into directories whose own encoding is still a prefix
/// of `name`, so it follows the one branch that can match rather than the tree.
///
/// **A partial match is unresolved, not a negative.** Descending a branch means
/// the leading components of `name` are real directories under `root`; failing
/// to find the leaf under them means the leaf is GONE, not that `name` names
/// something else. Worktrees are this case and are the majority of it: a
/// worktree directory is deleted when the worktree is, while `<repo>/.claude/
/// worktrees` survives empty, so every archived worktree session's project
/// directory has a live prefix and a dead leaf. Answering `Some(false)` there
/// would open exactly the transcripts D-06 folds into the excluded repo.
///
/// Unreadable entries and symlinks are unresolved for the same reason. A
/// symlinked branch is not followed - a link loop would not terminate - so when
/// one could still lead to `name` the answer is `None` rather than a negative
/// reached by not looking.
fn descends_to(root: &Path, name: &str) -> Option<bool> {
    if !root.is_dir() {
        return None;
    }
    let mut budget = DIR_SCAN_BUDGET;
    let mut stack = vec![root.to_path_buf()];
    let mut descended = false;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            budget = budget.checked_sub(1)?;
            let entry = entry.ok()?;
            let child = entry.path();
            let encoded = fold(&encode(&child.to_string_lossy()));
            let leads_to_name = name
                .strip_prefix(encoded.as_str())
                .is_some_and(|rest| rest.starts_with(ENCODED_SEPARATOR));
            if !encoded.eq(name) && !leads_to_name {
                continue;
            }
            // Only entries that could still be `name` are typed, so an
            // unreadable type on an unrelated file never decides anything.
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() {
                return None;
            }
            if !file_type.is_dir() {
                continue;
            }
            if encoded == name {
                return Some(true);
            }
            descended = true;
            stack.push(child);
        }
    }
    (!descended).then_some(false)
}

/// A path as its comparable components, folded for the platform.
fn components(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| fold(&c.as_os_str().to_string_lossy()))
        .collect()
}

/// Does `ancestor` cover `path`, and by how many components?
///
/// `Some(n)` when `ancestor` is `path` itself or a directory above it, where
/// `n` is how many components deep `ancestor` is - which is what makes "the
/// longest stored key that covers this directory" a comparison of numbers.
/// `None` when it covers nothing, including for an empty `ancestor`, which
/// would otherwise be a prefix of every path there is.
///
/// One rule, used twice on purpose. [`Config::excludes_path`] asks it whether a
/// project is hidden (D-23) and `recall::scope` asks it which project a working
/// directory sits in (D-12); two spellings of "is this path inside that one"
/// would eventually disagree about a trailing separator or about case folding,
/// and a project in scope under one rule and excluded under the other is a
/// search that returns an excluded project's turns.
pub fn covers(ancestor: &Path, path: &Path) -> Option<usize> {
    let prefix = components(ancestor);
    let candidate = components(path);
    (!prefix.is_empty()
        && candidate.len() >= prefix.len()
        && candidate[..prefix.len()] == prefix[..])
        .then_some(prefix.len())
}

/// Verbatim's config directory, resolved the way `store::data_dir` resolves the
/// data directory: an environment override first, then the platform location.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(dir) = non_empty_var(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(dir));
    }
    platform_config_dir()
}

#[cfg(not(target_os = "windows"))]
fn platform_config_dir() -> Result<PathBuf> {
    if let Some(xdg) = non_empty_var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(xdg).join("verbatim"));
    }
    Ok(home_dir()?.join(".config").join("verbatim"))
}

#[cfg(target_os = "windows")]
fn platform_config_dir() -> Result<PathBuf> {
    let appdata = non_empty_var("APPDATA").ok_or_else(|| Error::ConfigUnresolved {
        detail: format!("neither {CONFIG_DIR_ENV} nor APPDATA is set"),
    })?;
    Ok(PathBuf::from(appdata).join("verbatim"))
}

#[cfg(not(target_os = "windows"))]
fn home_dir() -> Result<PathBuf> {
    non_empty_var("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| Error::ConfigUnresolved {
            detail: "HOME is not set".into(),
        })
}

#[cfg(target_os = "windows")]
fn home_dir() -> Result<PathBuf> {
    non_empty_var("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::ConfigUnresolved {
            detail: "USERPROFILE is not set".into(),
        })
}

/// A configured string set to `""` is treated as unset, the way an empty
/// environment variable is ([`non_empty_var`]).
///
/// One rule for both sources, because the alternative is a `base_url = ""` that
/// resolves to a request against the empty string and an `api_key = ""` that
/// outranks a real key in the shared file for being "present".
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// The one place a credential can still reach a stream from a config file, shut
/// (PRIV-01).
///
/// A TOML parse error renders the offending line back at the reader:
///
/// ```text
/// TOML parse error at line 2, column 11
///   |
/// 2 | api_key = sk-not-a-quoted-string
///   |           ^
/// ```
///
/// Forgetting the quotes around a key is an ordinary mistake, and the reward
/// for making it must not be the key on stderr - `verbatim doctor` and every
/// command that loads a config would print it. When the parser's rendering
/// names `api_key` at all, its excerpt is dropped and only the position
/// survives, which is what the user needs to find the line in their own file.
///
/// Deliberately blunt: it withholds the excerpt whenever that string appears
/// anywhere in the error, including when the broken line is somewhere else
/// entirely. Withholding too much costs a reader one look at their own file;
/// withholding too little costs a key rotation.
fn withhold_secret_excerpt(detail: &str) -> String {
    if !detail.contains("api_key") {
        return detail.to_owned();
    }
    let position = detail.lines().next().unwrap_or_default();
    format!(
        "{position} (the parser's excerpt of the file is withheld here because it names api_key)"
    )
}

/// An environment variable set to the empty string is treated as unset: an
/// empty path would resolve to the process's current directory.
fn non_empty_var(name: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(name) {
        Some(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// The read side of ING-08: the one way a read path lists sessions.
///
/// **Every future read path is required to go through this module** rather than
/// querying `session_meta` directly. Phase 3's search, phase 5's injection and
/// the MCP tools all reuse it; a query that reaches past it is how an excluded
/// project becomes visible again.
///
/// The failure this exists to prevent is named in `.planning/PROJECT.md`: the
/// incumbent honors exclusion on write and ignores it on read. Verbatim honors
/// it on both, and honors it **retroactively** - which is why no per-session
/// flag is written at ingest (D-23). A flag would say what was true when the
/// session was archived, and the case that matters is precisely the session
/// archived *before* its project was excluded. The predicate is re-applied on
/// every read instead.
///
/// It is applied to `session_meta.project` **and** to
/// `session_meta.project_pre_worktree`, because either one alone leaks: a
/// worktree session carries the folded parent repo in `project` and the
/// worktree path in the pre-mapping column, and a user may reasonably exclude
/// either path.
///
/// A session whose `project` is null - one real transcript carries no `cwd` at
/// all - is visible, because nothing can say it is excluded.
pub mod visible {
    use rusqlite::Connection;

    use super::Config;
    use crate::error::Result;

    /// `project_pre_worktree`, or the literal `NULL` when the store predates it.
    ///
    /// The column arrived in phase 2 and `bring_forward` adds it - but only
    /// `Store::open` runs `bring_forward`, and the read commands deliberately
    /// open read-only (D-10). So there is an ordinary window where every read
    /// names a column the file does not have: upgrade the binary, run
    /// `verbatim search` before the next ingest. Naming it unconditionally made
    /// that window return a raw `no such column` from the first statement of
    /// both `search::run` and `context::window`, which is exactly the failure
    /// `Store::missing_columns` exists to prevent and the degraded read D-18
    /// asks for. There is no caller behaviour that could avoid it, since
    /// `scope::resolve` runs before anything else on both paths.
    ///
    /// Selecting `NULL` instead degrades the way the store itself has already
    /// degraded: a file with no pre-folding key has no pre-folding key to
    /// scope or exclude on, so every such session carries `None` and scoping
    /// falls back to `project` alone. Checked per call rather than cached
    /// because these are free functions over a borrowed connection;
    /// `pragma_table_info` is an in-memory lookup against the schema SQLite
    /// already parsed.
    pub(crate) fn pre_worktree_column(conn: &Connection) -> Result<&'static str> {
        let present: i64 = conn.query_row(
            "SELECT count(*) FROM pragma_table_info('session_meta')
              WHERE name = 'project_pre_worktree'",
            [],
            |r| r.get(0),
        )?;
        Ok(if present > 0 {
            "project_pre_worktree"
        } else {
            "NULL"
        })
    }

    /// One session a read path may see.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Session {
        /// The canonical transcript path, which is what `sessions` is keyed on.
        pub session_key: String,
        pub session_no: i64,
        pub project: Option<String>,
        pub project_pre_worktree: Option<String>,
        /// Turn rows this session contributed.
        pub turns: i64,
        /// The byte offset ingest will resume from, when one is recorded.
        pub watermark: Option<i64>,
    }

    /// What the visible sessions add up to.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct Counts {
        pub sessions: i64,
        pub turns: i64,
        pub watermarks: i64,
        /// Bytes those watermarks cover.
        pub watermark_bytes: i64,
    }

    /// Every session the config does not exclude, in ingest order.
    ///
    /// A `LEFT JOIN`, so a session archived without a `session_meta` row is
    /// still listed rather than silently dropped: `verbatim verify` is what
    /// reports that damage, and a read path that hid it would hide the evidence.
    pub fn sessions(conn: &Connection, config: &Config) -> Result<Vec<Session>> {
        let pre_worktree = match pre_worktree_column(conn)? {
            "project_pre_worktree" => "m.project_pre_worktree",
            absent => absent,
        };
        let mut statement = conn.prepare(&format!(
            "SELECT s.session_key, s.session_no, m.project, {pre_worktree},
                    (SELECT count(*) FROM turns t WHERE t.session_key = s.session_key),
                    (SELECT w.byte_offset FROM watermarks w
                      WHERE w.transcript_path = s.session_key)
             FROM sessions s LEFT JOIN session_meta m USING (session_key)
             ORDER BY s.session_no"
        ))?;
        let rows = statement.query_map([], |r| {
            Ok(Session {
                session_key: r.get(0)?,
                session_no: r.get(1)?,
                project: r.get(2)?,
                project_pre_worktree: r.get(3)?,
                turns: r.get(4)?,
                watermark: r.get(5)?,
            })
        })?;

        let mut out = Vec::new();
        for session in rows {
            let session = session?;
            if !is_excluded(config, &session) {
                out.push(session);
            }
        }
        Ok(out)
    }

    /// One distinct pair of project keys the archive carries, and what the
    /// config says about each of them.
    ///
    /// Both keys travel together rather than as two independent sets, because
    /// they are only meaningful as a pair: a longest-prefix hit on
    /// `project_pre_worktree` has to resolve back to that row's `project`
    /// before it can scope anything (phase 2 D-06 folded a repo and its
    /// worktree into ONE key on purpose), and exclusion has to be able to hide
    /// a session under its worktree path while leaving a sibling session with
    /// the same `project` visible.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ProjectKeys {
        /// The key ingest resolved, after worktree folding. `None` for a
        /// session whose records carry no `cwd` at all.
        pub project: Option<String>,
        /// The key the session had before that folding (ING-05).
        pub project_pre_worktree: Option<String>,
        /// The config excludes [`ProjectKeys::project`] itself.
        pub project_excluded: bool,
        /// The config excludes [`ProjectKeys::project_pre_worktree`].
        pub pre_worktree_excluded: bool,
    }

    impl ProjectKeys {
        /// Is a session carrying these keys hidden from every read path?
        pub fn excluded(&self) -> bool {
            self.project_excluded || self.pre_worktree_excluded
        }
    }

    /// Every distinct project key the archive carries, and which of them the
    /// config excludes.
    ///
    /// D-21: this is the projection a read path scopes and filters on, and
    /// [`sessions`] is not. Measured on a synthetic store shaped like the real
    /// one - 2,000 sessions, 250,000 turns, warm - `sessions()` costs 6.2-6.8
    /// ms because of its per-session `count(*)` on `turns` plus the watermark
    /// lookup, against 0.56-0.58 ms for the same join projecting only
    /// `project`. The whole phase 5 budget is single-digit milliseconds, so a
    /// search that paid for a turn count it never reads would spend the budget
    /// before the FTS query started and the cost would look like search.
    ///
    /// Read off `session_meta` alone. A session with no meta row carries no
    /// project key to scope on, so it contributes no row here - and it stays
    /// visible for exactly that reason, since nothing can say it is excluded.
    pub fn projects(conn: &Connection, config: &Config) -> Result<Vec<ProjectKeys>> {
        let pre_worktree = pre_worktree_column(conn)?;
        let mut statement = conn.prepare(&format!(
            "SELECT DISTINCT project, {pre_worktree} FROM session_meta
             ORDER BY project, 2"
        ))?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<String>>(1)?,
            ))
        })?;

        let excludes = |key: &Option<String>| {
            key.as_deref()
                .is_some_and(|k| config.excludes_path(std::path::Path::new(k)))
        };

        let mut out = Vec::new();
        for row in rows {
            let (project, project_pre_worktree) = row?;
            out.push(ProjectKeys {
                project_excluded: excludes(&project),
                pre_worktree_excluded: excludes(&project_pre_worktree),
                project,
                project_pre_worktree,
            });
        }
        Ok(out)
    }

    /// The counts `verbatim status` prints, over exactly the visible sessions.
    pub fn counts(conn: &Connection, config: &Config) -> Result<Counts> {
        let mut counts = Counts::default();
        for session in sessions(conn, config)? {
            counts.sessions += 1;
            counts.turns += session.turns;
            if let Some(offset) = session.watermark {
                counts.watermarks += 1;
                counts.watermark_bytes += offset;
            }
        }
        Ok(counts)
    }

    /// Is this session inside an excluded project, under either of its keys?
    pub fn is_excluded(config: &Config, session: &Session) -> bool {
        [&session.project, &session.project_pre_worktree]
            .into_iter()
            .flatten()
            .any(|key| config.excludes_path(std::path::Path::new(key)))
    }
}
