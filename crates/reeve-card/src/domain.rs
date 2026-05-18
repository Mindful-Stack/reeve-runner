//! Card domain entity.
//!
//! Stage 2: just re-exports the protocol DTO. Stage 5 introduces the
//! typed entity with private fields + invariants and converts at
//! handler/repo boundaries.

pub use reeve_protocol::Card;
