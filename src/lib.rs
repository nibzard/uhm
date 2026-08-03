//! Shared production primitives used by the CLI and its private benchmark helpers.
#![allow(dead_code)]

pub mod action;
pub mod api;
mod args;
mod cache;
pub mod capabilities;
mod clock;
mod command;
pub mod config;
mod context;
pub mod contract;
mod dirs;
mod doctor;
mod first_run;
mod history;
mod http;
mod input;
pub mod model_selection;
mod outcome;
mod parent_shell;
pub mod program;
pub mod prompt;
pub mod provider;
mod recovery;
mod render;
pub mod runtime;
mod safety;
mod secret;
pub mod shell;
pub mod shell_integration;
mod sse;
mod telemetry;
mod tty;
