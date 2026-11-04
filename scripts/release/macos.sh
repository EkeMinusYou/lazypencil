#!/bin/bash
# Sign before evidence collection; notarize the exact executable in the package.
set -euo pipefail
umask 077

preflight() {
  [[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_OS:-} == macOS ]] || return 1
  [[ ${RUNNER_ARCH:-} == ARM64 && ${RELEASE_PLATFORM:-} == macos-26 ]] || return 1
  local version
  version=$(jq -er '.version | select(test("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"))' resources/release.json)
  [[ ${GITHUB_REF:-} == "refs/tags/v$version" ]] || {
    echo 'Publish from the immutable tag matching resources/release.json.' >&2
    return 1
  }
  jq -e '.channel == "stable" and .target == "aarch64-apple-darwin"' resources/release.json >/dev/null
  : "${DEVELOPER_ID_P12_BASE64:?Missing signing certificate}"
  : "${DEVELOPER_ID_P12_PASSWORD:?Missing signing certificate password}"
  : "${APPLE_TEAM_ID:?Missing Apple team ID}"
  : "${APPLE_ID:?Missing notarization Apple Account}"
  : "${APPLE_APP_SPECIFIC_PASSWORD:?Missing Apple app-specific password}"
  [[ $APPLE_TEAM_ID =~ ^[A-Z0-9]{10}$ ]]
}

check_ci() {
  : "${GITHUB_REPOSITORY:?}" "${GITHUB_SHA:?}" "${GH_TOKEN:?}"
  gh api "repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/runs?head_sha=$GITHUB_SHA&status=success&event=push&branch=main&per_page=1" |
    jq -e --arg revision "$GITHUB_SHA" \
      '.workflow_runs | any(.head_sha == $revision and .head_branch == "main" and .event == "push" and .status == "completed" and .conclusion == "success")' >/dev/null || {
    echo 'Publication requires an existing successful main CI run for this exact commit.' >&2
    return 1
  }
}

cleanup() {
  if [[ ${credentials_search_list_changed:-false} == true ]]; then
    if (( ${#credentials_search_list[@]} > 0 )); then
      security list-keychains -d user -s "${credentials_search_list[@]}" || true
    else
      security list-keychains -d user -s || true
    fi
  fi
  if [[ -n ${credentials_keychain:-} ]]; then
    security delete-keychain "$credentials_keychain" || true
  fi
  if [[ -n ${credentials_dir:-} ]]; then
    rm -rf "$credentials_dir"
  fi
}

prepare_credentials() {
  local keychain_password keychain
  credentials_dir=$(mktemp -d "$RUNNER_TEMP/lazypencil-$1.XXXXXXXX")
  credentials_keychain="$credentials_dir/credentials.keychain-db"
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  security list-keychains -d user > "$credentials_dir/original-keychains.txt"
  credentials_search_list=()
  while IFS= read -r keychain; do
    keychain=${keychain#*\"}
    keychain=${keychain%\"*}
    if [[ -n $keychain ]]; then
      credentials_search_list+=("$keychain")
    fi
  done < "$credentials_dir/original-keychains.txt"
  keychain_password=$(openssl rand -hex 32)
  printf '::add-mask::%s\n' "$keychain_password"
  security create-keychain -p "$keychain_password" "$credentials_keychain"
  security set-keychain-settings -lut 21600 "$credentials_keychain"
  security unlock-keychain -p "$keychain_password" "$credentials_keychain"
  if [[ $1 == signing ]]; then
    printf '%s' "$DEVELOPER_ID_P12_BASE64" | base64 --decode > "$credentials_dir/certificate.p12"
    security import "$credentials_dir/certificate.p12" -P "$DEVELOPER_ID_P12_PASSWORD" \
      -k "$credentials_keychain" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
      -k "$keychain_password" "$credentials_keychain" >/dev/null
    credentials_search_list_changed=true
    if (( ${#credentials_search_list[@]} > 0 )); then
      security list-keychains -d user -s "$credentials_keychain" "${credentials_search_list[@]}"
    else
      security list-keychains -d user -s "$credentials_keychain"
    fi
  fi
}

sign_binary() {
  : "${DEVELOPER_ID_P12_BASE64:?}" "${DEVELOPER_ID_P12_PASSWORD:?}" "${APPLE_TEAM_ID:?}"
  local binary=$1 identity
  [[ -f $binary && ! -L $binary && $APPLE_TEAM_ID =~ ^[A-Z0-9]{10}$ ]] || return 1
  prepare_credentials signing
  identity=$(security find-identity -v -p codesigning "$credentials_keychain" |
    awk -v team="($APPLE_TEAM_ID)" '/Developer ID Application:/ && index($0, team) {print $2}')
  [[ $identity =~ ^[A-Fa-f0-9]{40}$ ]] || {
    echo 'Expected exactly one valid Developer ID Application identity for this team.' >&2
    return 1
  }
  /usr/bin/codesign --force --timestamp --options runtime \
    --identifier com.github.EkeMinusYou.lazypencil \
    --keychain "$credentials_keychain" --sign "$identity" "$binary"
  /usr/bin/codesign --verify --strict --verbose=2 "$binary"
}

notarize_binary() {
  : "${APPLE_ID:?}" "${APPLE_APP_SPECIFIC_PASSWORD:?}" "${APPLE_TEAM_ID:?}"
  local binary=$1 receipt=$2 result submission submitted=true
  [[ -f $binary && ! -L $binary ]] || return 1
  /usr/bin/codesign --verify --strict "$binary"
  prepare_credentials notary
  xcrun notarytool store-credentials lazypencil-notary \
    --apple-id "$APPLE_ID" --password "$APPLE_APP_SPECIFIC_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" --keychain "$credentials_keychain" >/dev/null
  mkdir "$credentials_dir/payload"
  cp "$binary" "$credentials_dir/payload/lazypencil"
  /usr/bin/ditto -c -k --keepParent "$credentials_dir/payload" "$credentials_dir/submission.zip"
  result="$credentials_dir/result.json"
  if ! xcrun notarytool submit "$credentials_dir/submission.zip" \
      --keychain-profile lazypencil-notary --keychain "$credentials_keychain" \
      --wait --timeout 20m --output-format json > "$result"; then
    submitted=false
    echo 'Notarization submission failed or timed out.' >&2
  fi
  submission=$(jq -er '.id | select(type == "string" and length > 0)' "$result")
  xcrun notarytool log "$submission" --keychain-profile lazypencil-notary \
    --keychain "$credentials_keychain" "${receipt%.json}.log.json"
  [[ $submitted == true ]] || return 1
  jq -e '.status == "Accepted"' "$result" >/dev/null || {
    echo "Notarization was not accepted; see ${receipt%.json}.log.json." >&2
    return 1
  }
  jq --arg sha256 "$(shasum -a 256 "$binary" | awk '{print $1}')" \
    '. + {binary_sha256: $sha256}' "$result" > "$receipt"
  # Raw Mach-O executables and tar archives cannot have tickets stapled to them.
  # Gatekeeper retrieves the ticket online; the notarization receipt ships alongside.
}

case ${1:-} in
  preflight) preflight ;;
  check-ci) check_ci ;;
  sign) sign_binary "$2" ;;
  notarize) notarize_binary "$2" "$3" ;;
  *) echo 'Usage: macos.sh preflight | check-ci | sign BINARY | notarize BINARY RECEIPT' >&2; exit 2 ;;
esac
