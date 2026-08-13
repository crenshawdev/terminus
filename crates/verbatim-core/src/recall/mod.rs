//! The query layer both terminal recall and the MCP server sit on.
//!
//! One raw string becomes ranked turns scoped to the project the process is
//! standing in, with excluded projects invisible, subagent turns present but
//! ranked below the turns the user watched, an excerpt cut from the archive, and
//! a chronological window around any hit. `verbatim search` (PLAN-3) and
//! `recall_search` (PLAN-4) are two front ends over exactly this, which is what
//! keeps them from drifting into two definitions of relevance.
//!
//! Nothing here writes. Every function takes a `&rusqlite::Connection` that the
//! caller opened - through [`crate::Store::open_read_only`] on a read command
//! (D-10) - rather than opening one of its own, so the layer cannot create a
//! store as a side effect of being asked a question.

pub mod context;
pub mod excerpt;
pub mod get;
pub mod query;
pub mod scope;
pub mod search;

pub use context::{ContextTurn, Window, MAX_CONTEXT_SIDE};
pub use excerpt::{Reads, EXCERPT_CHARS};
pub use get::{Absent, Fetched, Record};
pub use query::{EntityMatch, Query, MAX_QUERY_TOKENS};
pub use scope::{Reason, Scope};
pub use search::{Filters, Hit, Request, Response, DEFAULT_RESULTS, MAX_RESULTS};
