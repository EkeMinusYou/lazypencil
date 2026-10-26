//! Verify signed build provenance with the standard GitHub CLI verifier.
use crate::{dist, evidence, process, util};
use anyhow::{Context, Result, ensure};
use clap::{Args, ValueEnum};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Copy, ValueEnum)]
pub enum Kind {
    Source,
    Candidate,
}

impl Kind {
    fn workflow(self) -> &'static str {
        match self {
            Self::Source => ".github/workflows/ci.yml",
            Self::Candidate => ".github/workflows/release-evidence.yml",
        }
    }
}

#[derive(Args)]
pub struct Arguments {
    #[arg(long, value_enum)]
    pub kind: Kind,
    #[arg(long)]
    pub artifact: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
}

pub fn verification_command(
    artifact: &Path,
    kind: Kind,
    repository: &str,
    revision: &str,
    reference: &str,
) -> Result<Command> {
    ensure!(
        regex::Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")?.is_match(repository),
        "Invalid provenance repository"
    );
    ensure!(
        revision.len() == 40
            && revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "Invalid provenance source revision"
    );
    ensure!(
        reference.len() <= 1024
            && !reference
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace())
            && ["refs/heads/", "refs/tags/"].iter().any(|prefix| reference
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())),
        "Invalid provenance source ref"
    );
    let mut command = Command::new("gh");
    command.args(["attestation", "verify"]).arg(artifact).args([
        "--hostname",
        "github.com",
        "--repo",
        repository,
        "--signer-workflow",
        &format!("{repository}/{}", kind.workflow()),
        "--source-digest",
        revision,
        "--signer-digest",
        revision,
        "--source-ref",
        reference,
        "--predicate-type",
        "https://slsa.dev/provenance/v1",
        "--deny-self-hosted-runners",
        "--format",
        "json",
    ]);
    Ok(command)
}

impl Arguments {
    pub fn run(self) -> Result<()> {
        ensure!(
            std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true"),
            "Provenance verification requires the hosted workflow identity"
        );
        let artifact = std::path::absolute(&self.artifact)?;
        let output = std::path::absolute(&self.output)?;
        let parent = output
            .parent()
            .context("Provenance report needs a parent directory")?;
        let before = evidence::fingerprint(&artifact, "attested-artifact", dist::MAX_BYTES)?;
        let mut command = verification_command(
            &artifact,
            self.kind,
            &std::env::var("GITHUB_REPOSITORY")?,
            &std::env::var("GITHUB_SHA")?,
            &std::env::var("GITHUB_REF")?,
        )?;
        let result =
            process::output_limited(&mut command, Duration::from_secs(120), 4 * 1024 * 1024)?;
        fs::create_dir_all(parent)?;
        util::publish(&output.with_extension("stderr.log"), &result.stderr, false)?;
        ensure!(
            !result.timed_out && result.status.success(),
            "Signed provenance verification failed; see {}",
            output.with_extension("stderr.log").display()
        );
        let verified: Value = serde_json::from_slice(&result.stdout)?;
        ensure!(
            verified.as_array().is_some_and(|items| !items.is_empty()),
            "Missing verified provenance"
        );
        ensure!(
            evidence::fingerprint(&artifact, "attested-artifact", dist::MAX_BYTES)? == before,
            "Artifact changed during provenance verification"
        );
        util::publish(&output, &result.stdout, false)?;
        util::print(
            &serde_json::json!({"signed_provenance":"verified", "artifact":before, "workflow":self.kind.workflow(), "verification":output}),
        )
    }
}
