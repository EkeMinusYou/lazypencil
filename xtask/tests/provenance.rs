use lazypencil_xtask::ci::provenance::{Kind, verification_command};
use std::path::Path;

#[test]
fn provenance_verification_pins_both_workflow_and_source_identity_without_shell_interpretation() {
    let artifact = Path::new("/tmp/owned artifact;$(unused).tar.gz");
    let revision = "a".repeat(40);
    for (kind, workflow) in [
        (Kind::Source, "ci.yml"),
        (Kind::Candidate, "release-evidence.yml"),
    ] {
        let command =
            verification_command(artifact, kind, "owner/repo", &revision, "refs/heads/main")
                .unwrap();
        assert_eq!(command.get_program(), "gh");
        let arguments = command
            .get_args()
            .map(|s| s.to_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            &arguments[..3],
            ["attestation", "verify", artifact.to_str().unwrap()]
        );
        for (flag, expected) in [
            ("--hostname", "github.com"),
            ("--repo", "owner/repo"),
            ("--source-digest", revision.as_str()),
            ("--signer-digest", revision.as_str()),
            ("--source-ref", "refs/heads/main"),
            ("--predicate-type", "https://slsa.dev/provenance/v1"),
        ] {
            assert!(arguments.windows(2).any(|args| args == [flag, expected]));
        }
        let signer = format!("owner/repo/.github/workflows/{workflow}");
        assert!(
            arguments
                .windows(2)
                .any(|args| args == ["--signer-workflow", signer.as_str()])
        );
        assert!(arguments.contains(&"--deny-self-hosted-runners"));
        assert!(!arguments.iter().any(|arg| arg.contains("token=")));
    }
}

#[test]
fn missing_or_ambiguous_provenance_identity_cannot_fall_back_to_unpinned_verification() {
    let artifact = Path::new("/tmp/owned.tar.gz");
    let revision = "a".repeat(40);
    for repository in [
        "",
        "owner",
        "owner/repo/other",
        "owner repo/repo",
        "--repo owner/repo",
    ] {
        assert!(
            verification_command(
                artifact,
                Kind::Source,
                repository,
                &revision,
                "refs/heads/main"
            )
            .is_err()
        );
    }
    for revision in ["".into(), "a".repeat(39), "A".repeat(40), "g".repeat(40)] {
        assert!(
            verification_command(
                artifact,
                Kind::Source,
                "owner/repo",
                &revision,
                "refs/heads/main"
            )
            .is_err()
        );
    }
    for reference in [
        "",
        "main",
        "refs/heads/",
        "refs/pull/1/merge",
        "refs/tags/v1\n",
    ] {
        assert!(
            verification_command(artifact, Kind::Source, "owner/repo", &revision, reference)
                .is_err()
        );
    }
}
