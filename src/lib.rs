//! Shared document, editing, rendering, and agent services for lazypencil.

pub mod agent;
pub mod assets;
pub mod attachments;
pub mod cancel;
pub mod cdp;
pub mod cli;
#[cfg(feature = "tui")]
pub mod clipboard;
#[cfg(feature = "rendering")]
pub mod color;
pub mod config;
pub mod conversation;
pub mod doctor;
pub mod document;
pub mod documents;
#[cfg(feature = "rendering")]
pub mod dynamic;
pub mod error;
#[cfg(feature = "rendering")]
pub mod font_lock;
pub mod geometry;
pub mod guidance;
mod herdr;
pub mod icons;
#[cfg(feature = "rendering")]
pub mod image_geometry;
pub mod image_input;
mod inline_images;
#[cfg(feature = "rendering")]
pub mod layout;
mod managed;
pub mod mcp;
#[cfg(feature = "rendering")]
mod native_memory;
pub mod operations;
pub mod paths;
pub(crate) mod preview_control;
#[cfg(windows)]
mod process_job;
#[cfg(unix)]
mod process_owner;
pub mod project;
pub mod prompt;
mod query;
pub mod remote_images;
#[cfg(feature = "rendering")]
pub mod render;
#[cfg(feature = "rendering")]
mod retained;
pub mod runtime;
pub mod script;
pub mod service;
#[cfg(feature = "rendering")]
pub mod shader;
pub mod store;
#[cfg(feature = "rendering")]
pub mod text;
#[cfg(feature = "tui")]
pub mod tui;

pub use error::{Error, Result};

#[cfg(test)]
#[path = "../tests/support/fixture.rs"]
mod fixture_support;
