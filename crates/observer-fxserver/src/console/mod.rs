//! The server console: FXServer's output cut into lines, cleaned and stripped of secrets.

mod assemble;
mod mask;
mod sanitize;

pub use assemble::{Assembled, FLUSH_AFTER, LineAssembler, MAX_LINE};
pub use mask::mask;
pub use sanitize::{plain, sanitize};
