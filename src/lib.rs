//! Riffly: a block editor for quickly reading, reviewing and fixing
//! markdown, built on GPUI with Tiptap's command surface.

pub mod editor;
pub mod workspace;

pub use editor::RifflyEditor;
pub use workspace::Workspace;
