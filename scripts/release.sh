#!/usr/bin/env bash
# Builds, signs and publishes a Sedjem release that installed copies pick up
# through the in-app updater.
#
#   scripts/release.sh <version> [notes-file]
#
# Requirements (one-time, see README "Releasing"):
#   - "Sedjem Dev" code-signing identity in the login keychain (stable TCC grants)
#   - updater key at ~/.tauri/sedjem-updater.key, its password in the keychain
#     (service com.sedjem.release, account updater-key-password)
#   - gh authenticated with push access to the repo
#
# Releases are built locally on purpose: CI could not sign with the local
# identity, and a different signature would reset users' Accessibility grant
# on every update.
set -euo pipefail

REPO="enverkaradede/sedjem"
TARGET_PLATFORM="darwin-aarch64"
UPDATER_KEY="$HOME/.tauri/sedjem-updater.key"
KEYCHAIN_SERVICE="com.sedjem.release"
KEYCHAIN_ACCOUNT="updater-key-password"

VERSION="${1:?usage: scripts/release.sh <version> [notes-file]}"
NOTES_FILE="${2:-}"
TAG="v$VERSION"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

die() { printf '❌ %s\n' "$*" >&2; exit 1; }
step() { printf '\n▸ %s\n' "$*"; }

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be MAJOR.MINOR.PATCH, got '$VERSION'"
[[ -z "$NOTES_FILE" || -f "$NOTES_FILE" ]] || die "notes file not found: $NOTES_FILE"
for tool in gh pnpm cargo git security python3; do
  command -v "$tool" >/dev/null || die "missing tool: $tool"
done
[[ -f "$UPDATER_KEY" ]] || die "updater key missing at $UPDATER_KEY"
[[ "$(git rev-parse --abbrev-ref HEAD)" == "main" ]] || die "release from main only"
git diff --quiet && git diff --cached --quiet || die "working tree has uncommitted changes"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists"
gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1 && die "GitHub release $TAG already exists"

step "Setting version to $VERSION"
python3 - "$VERSION" <<'PY'
import json, re, sys
version = sys.argv[1]
for path in ("package.json", "src-tauri/tauri.conf.json"):
    with open(path) as f:
        data = json.load(f)
    data["version"] = version
    with open(path, "w") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
path = "src-tauri/Cargo.toml"
text = open(path).read()
text, count = re.subn(r'(\[package\][^\[]*?\nversion = ")[^"]+(")', rf'\g<1>{version}\g<2>', text, count=1)
assert count == 1, "could not find [package] version in Cargo.toml"
open(path, "w").write(text)
PY

step "Running the full test suite"
(cd src-tauri && cargo test --quiet && cargo clippy --all-targets --quiet -- -D warnings)
pnpm typecheck
pnpm test

step "Building and signing (app + dmg + updater artifacts)"
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(security find-generic-password -s "$KEYCHAIN_SERVICE" -a "$KEYCHAIN_ACCOUNT" -w)"
export TAURI_SIGNING_PRIVATE_KEY="$UPDATER_KEY" TAURI_SIGNING_PRIVATE_KEY_PASSWORD
trap 'unset TAURI_SIGNING_PRIVATE_KEY_PASSWORD' EXIT
pnpm tauri build --bundles app,dmg
unset TAURI_SIGNING_PRIVATE_KEY_PASSWORD

BUNDLE="src-tauri/target/release/bundle"
ARCHIVE="$BUNDLE/macos/Sedjem.app.tar.gz"
DMG="$(ls "$BUNDLE"/dmg/Sedjem_"$VERSION"_*.dmg)"
[[ -f "$ARCHIVE" && -f "$ARCHIVE.sig" && -f "$DMG" ]] || die "expected build artifacts are missing"

OUT="$(mktemp -d)"
ASSET="Sedjem_${VERSION}_aarch64.app.tar.gz"
cp "$ARCHIVE" "$OUT/$ASSET"
cp "$DMG" "$OUT/Sedjem_${VERSION}_aarch64.dmg"
NOTES="${NOTES_FILE:+$(cat "$NOTES_FILE")}"
NOTES="${NOTES:-Sedjem $TAG}"

step "Writing latest.json"
python3 - "$VERSION" "$NOTES" "$(cat "$ARCHIVE.sig")" \
  "https://github.com/$REPO/releases/download/$TAG/$ASSET" "$TARGET_PLATFORM" "$OUT/latest.json" <<'PY'
import datetime, json, sys
version, notes, signature, url, platform, out = sys.argv[1:7]
manifest = {
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {platform: {"signature": signature, "url": url}},
}
with open(out, "w") as f:
    json.dump(manifest, f, indent=2)
PY

step "Committing and tagging $TAG"
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git diff --cached --quiet || git commit -m "chore(release): $TAG"
git tag -a "$TAG" -m "Sedjem $TAG"
git push origin main --follow-tags

step "Publishing GitHub release"
gh release create "$TAG" --repo "$REPO" --title "Sedjem $TAG" --notes "$NOTES" --latest \
  "$OUT/Sedjem_${VERSION}_aarch64.dmg" "$OUT/$ASSET" "$OUT/latest.json"

printf '\n✅ Released %s — installed copies will offer the update on their next check.\n' "$TAG"
