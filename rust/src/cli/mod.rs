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

/// Prompts for a 1-based selection into a list of `len` items and returns the
/// 0-based index, or None (after printing the standard message) when the
/// selection is out of range. `default` is the 1-based default, if any.
pub fn prompt_index(prompt: &str, len: usize, default: Option<usize>) -> Result<Option<usize>> {
    let selection = prompt_usize(prompt, default)?;
    if selection >= 1 && selection <= len {
        Ok(Some(selection - 1))
    } else {
        println!("Invalid selection. Please try again.");
        Ok(None)
    }
}

pub fn confirm(prompt: &str) -> Result<bool> {
    Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .map_err(|e| CsError::Other(format!("Prompt failed: {e}")))
}
