//! Tools that check a server's resources and repair what they can, by hand or before every start. Each
//! tool reads the server folder, predicts what the FiveM client will do with it, and plans changes that
//! keep the game's behaviour; applying a plan replaces files atomically and keeps their originals, so
//! that every change can be healed or undone.

pub mod glob;
pub mod manifest;
