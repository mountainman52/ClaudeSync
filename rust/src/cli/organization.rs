use serde_json::Value;

use crate::cli::prompt_usize;
use crate::config::FileConfig;
use crate::error::Result;
use crate::utils::validate_and_get_provider;

pub fn ls(config: &FileConfig) -> Result<()> {
    let provider = validate_and_get_provider(config, false, false)?;
    let organizations = provider.get_organizations()?;
    if organizations.is_empty() {
        println!("No organizations with required capabilities (chat and claude_pro) found.");
    } else {
        println!("Available organizations with required capabilities:");
        for (idx, org) in organizations.iter().enumerate() {
            println!("  {}. {} (ID: {})", idx + 1, org.name, org.id);
        }
    }
    Ok(())
}

pub fn set(config: &mut FileConfig, org_id: Option<String>, provider_name: &str) -> Result<()> {
    config.set(
        "active_provider",
        Value::String(provider_name.to_string()),
        true,
    )?;

    let provider = validate_and_get_provider(config, false, false)?;
    let organizations = provider.get_organizations()?;

    if organizations.is_empty() {
        println!("No organizations with required capabilities found.");
        return Ok(());
    }

    if let Some(org_id) = org_id {
        match organizations.iter().find(|o| o.id == org_id) {
            Some(org) => {
                config.set(
                    "active_organization_id",
                    Value::String(org.id.clone()),
                    true,
                )?;
                println!("Selected organization: {} (ID: {})", org.name, org.id);
            }
            None => {
                println!("Organization with ID {org_id} not found.");
            }
        }
    } else {
        println!("Available organizations:");
        for (idx, org) in organizations.iter().enumerate() {
            println!("  {}. {} (ID: {})", idx + 1, org.name, org.id);
        }
        let selection = prompt_usize(
            "Enter the number of the organization you want to work with",
            Some(1),
        )?;
        if selection >= 1 && selection <= organizations.len() {
            let org = &organizations[selection - 1];
            config.set(
                "active_organization_id",
                Value::String(org.id.clone()),
                true,
            )?;
            println!("Selected organization: {} (ID: {})", org.name, org.id);
        } else {
            println!("Invalid selection. Please try again.");
        }
    }

    // Clear project-related settings when changing organization
    config.set("active_project_id", Value::Null, true)?;
    config.set("active_project_name", Value::Null, true)?;
    println!("Project settings cleared. Please select or create a new project for this organization.");
    Ok(())
}
