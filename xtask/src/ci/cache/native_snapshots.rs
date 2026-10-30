//! Keep the release baseline until a default build has extended its dependency graph.
use super::*;

fn validate(primary: &str) -> Result<()> {
    ensure!(
        primary.len() <= 300
            && Regex::new(r"^native-v2-[A-Za-z0-9_.-]+-[0-9a-f]{64}$")?.is_match(primary),
        "Invalid native dependency identity"
    );
    Ok(())
}

pub fn complete_key(primary: &str) -> Result<String> {
    validate(primary)?;
    Ok(format!("complete-{primary}"))
}

pub fn preferred_key(primary: &str, core_only: bool) -> Result<String> {
    validate(primary)?;
    if core_only {
        Ok(format!("core-{primary}"))
    } else {
        complete_key(primary)
    }
}

pub struct Publication<'a> {
    pub primary: &'a str,
    pub matched: &'a str,
    pub checkpoint: &'a str,
    pub core_only: bool,
    pub full_catalog: bool,
    pub release_verified: bool,
}

/// The unprefixed baseline has completed release dependencies. A complete
/// default catalog may extend that exact baseline, but a cold/legacy restore
/// cannot claim to contain the current release graph. Partial builds remain
/// resumable checkpoints. Cargo still decides freshness after every restore.
pub fn publication(input: Publication<'_>) -> Result<Option<String>> {
    let complete = complete_key(input.primary)?;
    let preferred = preferred_key(input.primary, input.core_only)?;
    let prefix = format!("checkpoint-{}-", input.primary);
    let suffix = input
        .checkpoint
        .strip_prefix(&prefix)
        .context("Dependency checkpoint belongs to another native identity")?;
    let parts = suffix.split('-').collect::<Vec<_>>();
    ensure!(parts.len() == 3, "Invalid dependency checkpoint");
    ensure!(
        checkpoint_key(input.primary, parts[0], parts[1], parts[2])? == input.checkpoint,
        "Invalid dependency checkpoint"
    );
    // Restoring the default/release graph does not prove that core's distinct
    // feature graph was cached. Retain its completed extension separately.
    if input.core_only {
        return Ok((input.matched != preferred).then_some(preferred));
    }
    if input.matched == complete {
        return Ok(None);
    }
    if input.matched == input.primary {
        return Ok(input.full_catalog.then_some(complete));
    }
    if input.release_verified {
        return Ok(Some(input.primary.to_owned()));
    }
    Ok(Some(input.checkpoint.to_owned()))
}
