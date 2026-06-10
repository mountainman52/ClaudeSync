use serde_json::{Map, Value};

use crate::config::FileConfig;
use crate::error::{CsError, Result};

/// Parses CLI string values into bool/number/string like the Python version.
fn coerce_value(value: &str) -> Value {
    match value.to_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = value.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = value.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(value.to_string())
}

pub fn set(config: &mut FileConfig, key: &str, value: &str) -> Result<()> {
    if !config.global_config.contains_key(key) && !config.local_config.contains_key(key) {
        return Err(CsError::Configuration(format!(
            "Configuration property '{key}' does not exist."
        )));
    }
    let coerced = coerce_value(value);
    let display = coerced.to_string();
    config.set(key, coerced, false)?;
    println!("Configuration {key} set to {}", display.trim_matches('"'));
    Ok(())
}

pub fn get(config: &FileConfig, key: &str) -> Result<()> {
    match config.get(key) {
        None | Some(Value::Null) => println!("Configuration {key} is not set"),
        Some(value) => match value {
            Value::String(s) => println!("{key}: {s}"),
            other => println!("{key}: {other}"),
        },
    }
    Ok(())
}

pub fn ls(config: &FileConfig) -> Result<()> {
    let mut combined: Map<String, Value> = config.global_config.clone();
    for (k, v) in &config.local_config {
        combined.insert(k.clone(), v.clone());
    }
    // serde_json maps are sorted by key, matching Python's sort_keys=True
    println!("{}", serde_json::to_string_pretty(&Value::Object(combined))?);
    Ok(())
}

// --- category subcommands ---

pub fn category_add(
    config: &mut FileConfig,
    name: &str,
    description: &str,
    patterns: Vec<String>,
) -> Result<()> {
    config.add_file_category(name, description, patterns)?;
    println!("File category '{name}' added successfully.");
    Ok(())
}

pub fn category_rm(config: &mut FileConfig, name: &str) -> Result<()> {
    config.remove_file_category(name)?;
    println!("File category '{name}' removed successfully.");
    Ok(())
}

pub fn category_update(
    config: &mut FileConfig,
    name: &str,
    description: Option<&str>,
    patterns: Option<Vec<String>>,
) -> Result<()> {
    config.update_file_category(name, description, patterns)?;
    println!("File category '{name}' updated successfully.");
    Ok(())
}

pub fn category_ls(config: &FileConfig) -> Result<()> {
    let categories = config.get("file_categories").unwrap_or(Value::Null);
    match categories.as_object().filter(|m| !m.is_empty()) {
        None => println!("No file categories defined."),
        Some(map) => {
            for (name, data) in map {
                println!("\nCategory: {name}");
                println!(
                    "Description: {}",
                    data.get("description").and_then(Value::as_str).unwrap_or("")
                );
                println!("Patterns:");
                if let Some(patterns) = data.get("patterns").and_then(Value::as_array) {
                    for pattern in patterns {
                        println!("  - {}", pattern.as_str().unwrap_or_default());
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn category_set_default(config: &mut FileConfig, category: &str) -> Result<()> {
    config.set_default_category(category)?;
    println!("Default sync category set to: {category}");
    Ok(())
}
