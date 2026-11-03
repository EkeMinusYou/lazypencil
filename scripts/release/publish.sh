#!/bin/bash
# Keep publication and cross-repository credentials out of the build job.
set -euo pipefail

version=$(jq -er '.version | select(test("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"))' resources/release.json)
[[ ${GITHUB_REF:-} == "refs/tags/v$version" ]]
stem="lazypencil-$version-aarch64-apple-darwin"
tag="v$version"
assets="${RUNNER_TEMP:?}/release-assets"

stage() {
  mkdir "$assets"
  xtask/target/debug/xtask dist verify-binary "$RUNNER_TEMP/lazypencil-candidate.tar.gz" > "$RUNNER_TEMP/release-package.json"
  jq -e --arg revision "$GITHUB_SHA" --arg version "$version" \
    '.release_ready == true and .source_revision == $revision and .package.version == $version and .target == "aarch64-apple-darwin"' \
    "$RUNNER_TEMP/release-package.json" >/dev/null
  jq -e --arg sha256 "$(jq -er '.binary_sha256' "$RUNNER_TEMP/release-package.json")" \
    '.status == "Accepted" and .binary_sha256 == $sha256' "$RUNNER_TEMP/notarization.json" >/dev/null
  cp "$RUNNER_TEMP/lazypencil-candidate.tar.gz" "$assets/$stem.tar.gz"
  cp "$RUNNER_TEMP/notarization.json" "$assets/$stem.notarization.json"
  xtask/target/debug/xtask dist homebrew "$assets/$stem.tar.gz" --output "$assets/lazypencil.rb"
}

finalize() {
  cp "$ATTESTATION_BUNDLE" "$assets/$stem.sigstore.json"
  (
    cd "$assets"
    shasum -a 256 "$stem.tar.gz" "$stem.sigstore.json" "$stem.notarization.json" > "lazypencil-$version-SHA256SUMS"
  )
}

publish() {
  (
    cd "$assets"
    shasum -a 256 --check "lazypencil-$version-SHA256SUMS"
  )
  # Never replace an existing release or its assets. A failed upload leaves a
  # draft for inspection; remove that draft explicitly before retrying.
  gh release create "$tag" --repo "$GITHUB_REPOSITORY" --verify-tag --draft \
    --title "$tag" --generate-notes \
    "$assets/$stem.tar.gz" "$assets/$stem.sigstore.json" \
    "$assets/$stem.notarization.json" "$assets/lazypencil-$version-SHA256SUMS"
  gh release edit "$tag" --repo "$GITHUB_REPOSITORY" --draft=false --latest
}

update_tap() {
  local repository=EkeMinusYou/homebrew-lazypencil previous_sha previous_version newest request
  : "${GH_TOKEN:?Missing repository-scoped GitHub App token}"
  [[ -s $assets/lazypencil.rb ]]
  previous_sha=$(gh api "repos/$repository/git/trees/main?recursive=1" \
    --jq '.tree[] | select(.path == "Casks/lazypencil.rb") | .sha')
  if [[ -n $previous_sha ]]; then
    gh api "repos/$repository/git/blobs/$previous_sha" --jq .content | base64 --decode > "$RUNNER_TEMP/previous-cask.rb"
    previous_version=$(sed -nE 's/^  version "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' "$RUNNER_TEMP/previous-cask.rb")
    [[ -n $previous_version ]] || { echo 'Cannot determine the installed Cask version.' >&2; return 1; }
    if [[ $previous_version == "$version" ]]; then
      cmp "$RUNNER_TEMP/previous-cask.rb" "$assets/lazypencil.rb"
      return
    fi
    newest=$(printf '%s\n%s\n' "$previous_version" "$version" | sort -V | tail -n 1)
    [[ $newest == "$version" ]] || { echo 'Refusing to downgrade the Homebrew Cask.' >&2; return 1; }
  fi
  request="$RUNNER_TEMP/tap-update.json"
  jq -n --arg message "Update lazypencil to $version" \
    --arg content "$(base64 < "$assets/lazypencil.rb" | tr -d '\n')" --arg sha "$previous_sha" \
    '{message: $message, content: $content, branch: "main"} + (if $sha == "" then {} else {sha: $sha} end)' > "$request"
  # The existing blob SHA makes concurrent writes fail rather than overwrite.
  gh api --method PUT "repos/$repository/contents/Casks/lazypencil.rb" --input "$request" --silent
}

case ${1:-} in
  stage) stage ;;
  finalize) finalize ;;
  publish) publish ;;
  update-tap) update_tap ;;
  *) echo 'Usage: publish.sh stage | finalize | publish | update-tap' >&2; exit 2 ;;
esac
