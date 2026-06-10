pub mod auth;
pub mod chat;
pub mod config_cmd;
pub mod organization;
pub mod project;
pub mod push;
pub mod schedule;
pub mod session;
pub mod submodule;

use dialoguer::{Confirm, Input, Password};

use crate::error::{CsError, Result};

pub fn prompt_string(prompt: &str, default: Option<&str>) -> Result<String> {
    let mut input = Input::<String>::new().with_prompt(prompt);
    if let Some(d) = default {
        input = input.default(d.to_string());
    }
    input
        .interact_text()
        .map_err(|e| CsError::Other(format!("Prompt failed: {e}")))
}

pub fn prompt_usize(prompt: &str, default: Option<usize>) -> Result<usize> {
    let mut input = Input::<usize>::new().with_prompt(prompt);
    if let Some(d) = default {
        input = input.default(d);
    }
    input
        .interact_text()
        .map_err(|e| CsError::Other(format!("Prompt failed: {e}")))
}

pub fn prompt_password(prompt: &str) -> Result<String> {
    Password::new()
        .with_prompt(prompt)
        .interact()
        .map_err(|e| CsError::Other(format!("Prompt failed: {e}")))
}

pub fn confirm(prompt: &str) -> Result<bool> {
    Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .map_err(|e| CsError::Other(format!("Prompt failed: {e}")))
}
