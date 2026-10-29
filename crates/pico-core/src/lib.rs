//! Core of Pico: everything that does not need a window.
//!
//! Formats follow Picasa 3 so existing libraries keep their captions, stars and
//! edits. See `docs/research/picasa3-analysis.md` for where each detail comes from.

pub mod adjust;
pub mod agree;
pub mod album_file;
pub mod collage;
pub mod config;
pub mod effects;
pub mod exif;
mod exif_tags;
pub mod export;
pub mod fileops;
pub mod filters;
pub mod folders;
pub mod geo;
pub mod ini;
pub mod library;
mod maker_notes;
mod maker_tags;
pub mod media;
pub mod meta;
pub mod metadata;
pub mod movie;
pub mod ops;
pub mod rect64;
pub mod render;
pub mod thumbnail;
pub mod video;
pub mod xmp;

/// Re-exported so the app uses the same `image` version as the core.
pub use image;
