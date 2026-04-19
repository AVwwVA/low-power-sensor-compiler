#[allow(clippy::module_inception)]
pub mod lexer;
pub mod tokens;

pub use lexer::{Lexer, SpannedToken};
