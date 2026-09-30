//! Adapter for the owned runtimed engine over its Varlink socket.
//!
//! Provider kind `"runtimed"`, base URL is the Runtime1 socket path
//! (`/run/syntrop/io.syntrop.Runtime1`). Calls `Generate` for completions;
//! runtimed is single-shot, so the streaming half answers with one SSE
//! chunk followed by `[DONE]`.

pub mod chat_completion;
pub mod chat_stream;
pub mod engine_call;
pub mod probe;

pub use engine_call::RuntimedAdapter;
