//! Core modules for the `molenest` desktop application.
//!
//! The crate keeps SSH configuration parsing, command construction, and process
//! management separate from the Slint UI so those behaviors remain testable.

pub mod app;
pub mod config;
pub mod paths;
pub mod process;
pub mod ssh;
