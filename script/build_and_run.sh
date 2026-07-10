#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-run}"
APP_NAME="Murmur"
MIN_SYSTEM_VERSION="14.0"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RESOURCE_DIR="$ROOT_DIR/Resources"
BUILD_CONFIG="${MURMUR_BUILD_CONFIG:-release}"
DIST_DIR="${MURMUR_DIST_DIR:-$HOME/Applications/MurmurDist}"
APP_BUNDLE="$DIST_DIR/$APP_NAME.app"
APP_CONTENTS="$APP_BUNDLE/Contents"
APP_MACOS="$APP_CONTENTS/MacOS"
APP_RESOURCES="$APP_CONTENTS/Resources"
APP_BINARY="$APP_MACOS/$APP_NAME"
INFO_PLIST="$APP_CONTENTS/Info.plist"
ENTITLEMENTS_PLIST="${MURMUR_ENTITLEMENTS_PLIST:-$RESOURCE_DIR/Entitlements.plist}"
ARCHIVE_DIR="${MURMUR_ARCHIVE_DIR:-$DIST_DIR/archive}"

BUNDLE_ID="${MURMUR_BUNDLE_ID:-com.pais.murmur}"
APP_VERSION="${MURMUR_APP_VERSION:-0.1.0}"
APP_BUILD="${MURMUR_APP_BUILD:-1}"

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

detect_codesign_identity() {
  if [[ -n "${MURMUR_CODESIGN_IDENTITY:-}" ]]; then
    printf '%s\n' "$MURMUR_CODESIGN_IDENTITY"
    return 0
  fi

  local identities
  identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"

  if printf '%s' "$identities" | grep -q '"Murmur Dev"'; then
    printf '%s\n' "Murmur Dev"
    return 0
  fi

  local apple_dev
  apple_dev="$(printf '%s' "$identities" | grep -o '"Apple Development: [^"]*"' | head -1 | tr -d '"')"
  if [[ -n "$apple_dev" ]]; then
    printf '%s\n' "$apple_dev"
    return 0
  fi

  printf '%s\n' "-"
}

CODESIGN_IDENTITY="$(detect_codesign_identity)"

if [[ "$CODESIGN_IDENTITY" == "-" ]]; then
  printf 'warning: signing ad-hoc. Every rebuild resets the Accessibility grant in\n' >&2
  printf 'warning: System Settings (the toggle looks ON but the app is not trusted).\n' >&2
  printf 'warning: Create a self-signed "Murmur Dev" certificate or set MURMUR_CODESIGN_IDENTITY.\n' >&2
  printf 'warning: See BUILD.md "Stable local signing".\n' >&2
fi

artifact_arch() {
  case "$(uname -m)" in
    arm64)
      printf '%s\n' "aarch64"
      ;;
    x86_64)
      printf '%s\n' "x86_64"
      ;;
    *)
      uname -m
      ;;
  esac
}

ARTIFACT_ARCH="$(artifact_arch)"
ZIP_PATH="$ARCHIVE_DIR/${APP_NAME}_${APP_VERSION}_${ARTIFACT_ARCH}.app.zip"
DMG_PATH="$ARCHIVE_DIR/${APP_NAME}_${APP_VERSION}_${ARTIFACT_ARCH}.dmg"
NOTARY_ARGS=()

native_bundle_realpath() {
  /bin/realpath "$APP_BUNDLE" 2>/dev/null || printf '%s\n' "$APP_BUNDLE"
}

native_pid() {
  local bundle_path
  local bundle_realpath
  bundle_path="$APP_BUNDLE"
  bundle_realpath="$(native_bundle_realpath)"

  while IFS= read -r pid; do
    local command_path
    command_path="$(ps -p "$pid" -ww -o command= 2>/dev/null || true)"
    case "$command_path" in
      "$bundle_path"/Contents/MacOS/"$APP_NAME"|"$bundle_path"/Contents/MacOS/"$APP_NAME"\ *|"$bundle_realpath"/Contents/MacOS/"$APP_NAME"|"$bundle_realpath"/Contents/MacOS/"$APP_NAME"\ *)
        printf '%s\n' "$pid"
        return 0
        ;;
    esac
  done < <(pgrep -x "$APP_NAME" 2>/dev/null || true)

  return 1
}

terminate_existing_native() {
  local pid
  while IFS= read -r pid; do
    kill "$pid" >/dev/null 2>&1 || true
  done < <(native_pid || true)

  local attempt
  for attempt in {1..40}; do
    if ! native_pid >/dev/null; then
      return 0
    fi
    sleep 0.25
  done
}

wait_for_native() {
  local attempt
  for attempt in {1..20}; do
    if native_pid >/dev/null; then
      return 0
    fi
    sleep 0.25
  done

  return 1
}

clean_bundle_xattrs() {
  /usr/bin/xattr -c -r -s "$APP_BUNDLE" >/dev/null 2>&1 || true
  /usr/bin/xattr -d -s com.apple.FinderInfo "$APP_BUNDLE" >/dev/null 2>&1 || true
  /usr/bin/xattr -d -s 'com.apple.fileprovider.fpfs#P' "$APP_BUNDLE" >/dev/null 2>&1 || true
}

create_archives() {
  local dmg_source_dir="$DIST_DIR/dmg-src"

  rm -rf "$ARCHIVE_DIR" "$dmg_source_dir"
  mkdir -p "$ARCHIVE_DIR" "$dmg_source_dir"

  /usr/bin/ditto \
    -c -k --keepParent --norsrc --noextattr --noqtn \
    "$APP_BUNDLE" \
    "$ZIP_PATH"

  /usr/bin/ditto --norsrc --noextattr --noqtn "$APP_BUNDLE" "$dmg_source_dir/$APP_NAME.app"
  ln -s /Applications "$dmg_source_dir/Applications"
  /usr/bin/hdiutil create \
    -volname "$APP_NAME" \
    -srcfolder "$dmg_source_dir" \
    -ov \
    -format UDZO \
    "$DMG_PATH" >/dev/null
  rm -rf "$dmg_source_dir"
}

configure_notary_args() {
  NOTARY_ARGS=()

  if [[ -n "${MURMUR_NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
    NOTARY_ARGS=(--keychain-profile "$MURMUR_NOTARY_KEYCHAIN_PROFILE")
    return 0
  fi

  if [[ -n "${MURMUR_NOTARY_APPLE_ID:-}" && -n "${MURMUR_NOTARY_TEAM_ID:-}" && -n "${MURMUR_NOTARY_PASSWORD:-}" ]]; then
    NOTARY_ARGS=(--apple-id "$MURMUR_NOTARY_APPLE_ID" --team-id "$MURMUR_NOTARY_TEAM_ID" --password "$MURMUR_NOTARY_PASSWORD")
    return 0
  fi

  fail "notarization requires MURMUR_NOTARY_KEYCHAIN_PROFILE or MURMUR_NOTARY_APPLE_ID, MURMUR_NOTARY_TEAM_ID, and MURMUR_NOTARY_PASSWORD"
}

require_developer_id_identity() {
  if [[ "$CODESIGN_IDENTITY" == "-" ]]; then
    fail "notarization requires MURMUR_CODESIGN_IDENTITY to be a Developer ID Application identity"
  fi

  if [[ "${MURMUR_ALLOW_NON_DEVELOPER_ID_NOTARIZATION:-0}" == "1" ]]; then
    return 0
  fi

  if ! /usr/bin/security find-identity -p codesigning -v 2>/dev/null \
    | /usr/bin/grep -F "$CODESIGN_IDENTITY" \
    | /usr/bin/grep -q "Developer ID Application"; then
    fail "notarization requires a Developer ID Application identity; set MURMUR_CODESIGN_IDENTITY or MURMUR_ALLOW_NON_DEVELOPER_ID_NOTARIZATION=1 for local tool testing"
  fi
}

require_notarization_tools() {
  /usr/bin/xcrun --find notarytool >/dev/null 2>&1 || fail "xcrun notarytool is required for notarization"
  /usr/bin/xcrun --find stapler >/dev/null 2>&1 || fail "xcrun stapler is required for notarization"
}

require_notarization_prerequisites() {
  require_developer_id_identity
  require_notarization_tools
  configure_notary_args
}

notarize_artifact() {
  local artifact_path="$1"
  /usr/bin/xcrun notarytool submit "$artifact_path" --wait "${NOTARY_ARGS[@]}"
}

staple_artifact() {
  local artifact_path="$1"
  /usr/bin/xcrun stapler staple "$artifact_path"
  /usr/bin/xcrun stapler validate "$artifact_path"
}

sign_disk_image() {
  /usr/bin/codesign --force --sign "$CODESIGN_IDENTITY" "$DMG_PATH" >/dev/null
  /usr/bin/codesign --verify --verbose=2 "$DMG_PATH" >/dev/null
}

create_notarized_archives() {
  create_archives
  notarize_artifact "$ZIP_PATH"
  staple_artifact "$APP_BUNDLE"
  create_archives
  sign_disk_image
  notarize_artifact "$DMG_PATH"
  staple_artifact "$DMG_PATH"
  /usr/sbin/spctl --assess --type execute --verbose=2 "$APP_BUNDLE"
  /usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG_PATH"
}

case "$MODE" in
  --notarize|notarize)
    require_notarization_prerequisites
    ;;
esac

terminate_existing_native

printf 'build configuration: %s\n' "$BUILD_CONFIG"
swift build -c "$BUILD_CONFIG" -debug-info-format none --product "$APP_NAME"
BUILD_BINARY="$(swift build -c "$BUILD_CONFIG" -debug-info-format none --show-bin-path)/$APP_NAME"

rm -rf "$APP_BUNDLE"
mkdir -p "$APP_MACOS" "$APP_RESOURCES"
cp "$BUILD_BINARY" "$APP_BINARY"
chmod +x "$APP_BINARY"

if [[ -f "$RESOURCE_DIR/Murmur.icns" ]]; then
  cp "$RESOURCE_DIR/Murmur.icns" "$APP_RESOURCES/Murmur.icns"
fi

for sound in marimba_start.wav marimba_stop.wav pop_start.wav pop_stop.wav; do
  if [[ -f "$RESOURCE_DIR/$sound" ]]; then
    cp "$RESOURCE_DIR/$sound" "$APP_RESOURCES/$sound"
  fi
done

FONT_DIR="$RESOURCE_DIR/Fonts"
if [[ -d "$FONT_DIR" ]]; then
  cp "$FONT_DIR"/dm-sans-latin-wght-normal.woff2 "$APP_RESOURCES/" 2>/dev/null || true
  cp "$FONT_DIR"/dm-sans-latin-wght-italic.woff2 "$APP_RESOURCES/" 2>/dev/null || true
  cp "$FONT_DIR"/dm-sans-latin-ext-wght-normal.woff2 "$APP_RESOURCES/" 2>/dev/null || true
  cp "$FONT_DIR"/dm-sans-latin-ext-wght-italic.woff2 "$APP_RESOURCES/" 2>/dev/null || true
fi

if [[ -d "$RESOURCE_DIR" ]]; then
  cp "$RESOURCE_DIR"/MurmurTextLogo*.png "$APP_RESOURCES/" 2>/dev/null || true
fi

cat >"$INFO_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>$APP_NAME</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_ID</string>
  <key>CFBundleName</key>
  <string>$APP_NAME</string>
  <key>CFBundleDisplayName</key>
  <string>$APP_NAME</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$APP_VERSION</string>
  <key>CFBundleVersion</key>
  <string>$APP_BUILD</string>
  <key>CFBundleIconFile</key>
  <string>Murmur</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.productivity</string>
  <key>LSMinimumSystemVersion</key>
  <string>$MIN_SYSTEM_VERSION</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSPrincipalClass</key>
  <string>NSApplication</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Murmur needs microphone access to transcribe audio locally.</string>
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>Murmur needs speech recognition access to transcribe recorded audio.</string>
</dict>
</plist>
PLIST

clean_bundle_xattrs
codesign_args=(--force --sign "$CODESIGN_IDENTITY" --options runtime)
if [[ -f "$ENTITLEMENTS_PLIST" ]]; then
  codesign_args+=(--entitlements "$ENTITLEMENTS_PLIST")
fi
/usr/bin/codesign "${codesign_args[@]}" "$APP_BUNDLE" >/dev/null
/usr/bin/codesign --verify --deep --strict "$APP_BUNDLE" >/dev/null

open_app() {
  local background="${1:-0}"
  shift || true

  local open_args=(-n)
  if [[ "$background" == "1" ]]; then
    open_args+=(-g)
  fi

  if [[ "${MURMUR_PORTABLE_SMOKE:-0}" == "1" ]]; then
    local smoke_data_dir="$DIST_DIR/smoke-data"
    mkdir -p "$smoke_data_dir"
    open_args+=(--env "MURMUR_APP_DATA_DIR=$smoke_data_dir")
  fi

  open_args+=(-a "$APP_BUNDLE")
  if [[ "$#" -gt 0 ]]; then
    open_args+=(--args "$@")
  fi

  /usr/bin/open "${open_args[@]}"
}

run_readiness_command() {
  local name="$1"
  shift

  local stdout_path="$READINESS_DIR/$name.stdout"
  local stderr_path="$READINESS_DIR/$name.stderr"
  printf 'readiness: %s\n' "$name"
  if MURMUR_APP_DATA_DIR="$READINESS_DATA_DIR" "$APP_BINARY" "$@" >"$stdout_path" 2>"$stderr_path"; then
    if [[ -s "$stdout_path" && ! -f "$READINESS_DIR/$name.json" ]]; then
      cp "$stdout_path" "$READINESS_DIR/$name.json"
    fi
    return 0
  fi

  printf 'readiness failed: %s (stdout: %s, stderr: %s)\n' "$name" "$stdout_path" "$stderr_path" >&2
  return 1
}

readiness_bool() {
  if [[ "$1" == "1" ]]; then
    printf 'true'
  else
    printf 'false'
  fi
}

write_readiness_notarization_status() {
  local developer_id_identity_present=0
  local notarytool_present=0
  local stapler_present=0
  local keychain_profile_present=0
  local env_credentials_present=0
  local can_notarize=0

  printf 'readiness: notarization-prerequisites\n'

  if /usr/bin/security find-identity -p codesigning -v 2>/dev/null | /usr/bin/grep -q "Developer ID Application"; then
    developer_id_identity_present=1
  fi
  if /usr/bin/xcrun --find notarytool >/dev/null 2>&1; then
    notarytool_present=1
  fi
  if /usr/bin/xcrun --find stapler >/dev/null 2>&1; then
    stapler_present=1
  fi
  if [[ -n "${MURMUR_NOTARY_KEYCHAIN_PROFILE:-}" ]]; then
    keychain_profile_present=1
  fi
  if [[ -n "${MURMUR_NOTARY_APPLE_ID:-}" && -n "${MURMUR_NOTARY_TEAM_ID:-}" && -n "${MURMUR_NOTARY_PASSWORD:-}" ]]; then
    env_credentials_present=1
  fi
  if [[ "$developer_id_identity_present" == "1" &&
    "$notarytool_present" == "1" &&
    "$stapler_present" == "1" &&
    ("$keychain_profile_present" == "1" || "$env_credentials_present" == "1") ]]; then
    can_notarize=1
  fi

  cat >"$READINESS_DIR/notarization-prerequisites.json" <<JSON
{"canNotarize":$(readiness_bool "$can_notarize"),"developerIDIdentityPresent":$(readiness_bool "$developer_id_identity_present"),"envCredentialsPresent":$(readiness_bool "$env_credentials_present"),"keychainProfilePresent":$(readiness_bool "$keychain_profile_present"),"notarytoolPresent":$(readiness_bool "$notarytool_present"),"staplerPresent":$(readiness_bool "$stapler_present"),"success":true}
JSON
}

write_json_bool() {
  local key="$1"
  local value="$2"
  printf '"%s":%s' "$key" "$(readiness_bool "$value")"
}

run_release_readiness() {
  local release_dir="$ARCHIVE_DIR/release-readiness"
  local zip_check_dir="$release_dir/zip-check"
  local dmg_attach_output="$release_dir/dmg-attach.txt"
  local zip_signature_valid=0
  local dmg_signature_valid=0
  local dmg_verify_valid=0
  local dmg_contains_app=0
  local dmg_contains_applications_link=0
  local app_signature_valid=0
  local spctl_accepted=0
  local dmg_mount_point=""
  local dmg_device=""
  local release_success=0

  rm -rf "$release_dir"
  mkdir -p "$zip_check_dir"

  create_archives

  if /usr/bin/codesign --verify --deep --strict "$APP_BUNDLE" >/dev/null 2>&1; then
    app_signature_valid=1
  fi

  /usr/bin/ditto -x -k "$ZIP_PATH" "$zip_check_dir"
  if /usr/bin/codesign --verify --deep --strict "$zip_check_dir/$APP_NAME.app" >/dev/null 2>&1; then
    zip_signature_valid=1
  fi

  if /usr/bin/hdiutil verify "$DMG_PATH" >/dev/null 2>&1; then
    dmg_verify_valid=1
  fi

  /usr/bin/hdiutil attach -readonly -nobrowse "$DMG_PATH" >"$dmg_attach_output"
  dmg_mount_point="$(/usr/bin/awk '/\/Volumes\// { print $NF; exit }' "$dmg_attach_output")"
  dmg_device="$(/usr/bin/awk '/\/Volumes\// { print $1; exit }' "$dmg_attach_output")"
  if [[ -n "$dmg_mount_point" && -d "$dmg_mount_point/$APP_NAME.app" ]]; then
    dmg_contains_app=1
    if /usr/bin/codesign --verify --deep --strict "$dmg_mount_point/$APP_NAME.app" >/dev/null 2>&1; then
      dmg_signature_valid=1
    fi
  fi
  if [[ -n "$dmg_mount_point" && -L "$dmg_mount_point/Applications" ]]; then
    dmg_contains_applications_link=1
  fi
  if [[ -n "$dmg_device" ]]; then
    /usr/bin/hdiutil detach "$dmg_device" >/dev/null 2>&1 || true
  fi

  if /usr/sbin/spctl --assess --type execute --verbose=4 "$APP_BUNDLE" >/dev/null 2>&1; then
    spctl_accepted=1
  fi

  if [[ "$app_signature_valid" -eq 1 &&
    "$zip_signature_valid" -eq 1 &&
    "$dmg_verify_valid" -eq 1 &&
    "$dmg_contains_app" -eq 1 &&
    "$dmg_contains_applications_link" -eq 1 &&
    "$dmg_signature_valid" -eq 1 ]]; then
    release_success=1
  fi

  cat >"$release_dir/release-readiness.json" <<JSON
{$(write_json_bool "appSignatureValid" "$app_signature_valid"),$(write_json_bool "zipSignatureValid" "$zip_signature_valid"),$(write_json_bool "dmgVerifyValid" "$dmg_verify_valid"),$(write_json_bool "dmgContainsApp" "$dmg_contains_app"),$(write_json_bool "dmgContainsApplicationsLink" "$dmg_contains_applications_link"),$(write_json_bool "dmgSignatureValid" "$dmg_signature_valid"),$(write_json_bool "spctlAccepted" "$spctl_accepted"),"appBundle":"$APP_BUNDLE","zipPath":"$ZIP_PATH","dmgPath":"$DMG_PATH","spctlExpectedWithoutNotarization":true,$(write_json_bool "success" "$release_success")}
JSON

  printf 'release readiness output: %s\n' "$release_dir/release-readiness.json"
  if [[ "$release_success" -ne 1 ]]; then
    fail "release readiness failed; see $release_dir/release-readiness.json"
  fi
}

run_readiness_smokes() {
  READINESS_DIR="$DIST_DIR/readiness"
  READINESS_DATA_DIR="$DIST_DIR/readiness-data"
  rm -rf "$READINESS_DIR" "$READINESS_DATA_DIR"
  mkdir -p "$READINESS_DIR" "$READINESS_DATA_DIR"

  clean_bundle_xattrs
  /usr/bin/codesign --verify --deep --strict "$APP_BUNDLE" >/dev/null

  local failed=0

  run_readiness_command \
    permission-status \
    --smoke-permission-status \
    --smoke-output-json "$READINESS_DIR/permission-status.json" || failed=1

  run_readiness_command \
    replacement-readiness \
    --smoke-replacement-readiness \
    --smoke-output-json "$READINESS_DIR/replacement-readiness.json" || failed=1

  run_readiness_command \
    overlay-visible \
    --start-hidden \
    --smoke-overlay-state recording \
    --smoke-output-json "$READINESS_DIR/overlay-visible.json" \
    --smoke-output-image "$READINESS_DIR/overlay-visible.png" || failed=1

  run_readiness_command \
    remote-control-launchservices \
    --smoke-remote-control-listener \
    --smoke-remote-control-launchservices \
    --smoke-remote-control-command=cancel \
    --smoke-output-json "$READINESS_DIR/remote-control-launchservices.json" || failed=1

  run_readiness_command \
    model-cache-status \
    --smoke-model-cache-status tiny || failed=1

  if [[ "${MURMUR_READINESS_AUDIO_RECORDING_SMOKE:-0}" == "1" ]]; then
    local -a audio_recording_args=(
      --smoke-record-audio "$READINESS_DIR/audio-recording.wav"
      --smoke-record-duration-ms "${MURMUR_READINESS_RECORD_DURATION_MS:-1500}"
      --smoke-output-json "$READINESS_DIR/audio-recording.json"
    )
    if [[ -n "${MURMUR_READINESS_MICROPHONE:-}" ]]; then
      audio_recording_args+=(--smoke-record-microphone "$MURMUR_READINESS_MICROPHONE")
    fi

    run_readiness_command audio-recording "${audio_recording_args[@]}" || failed=1

    if [[ "${MURMUR_READINESS_REQUIRE_VOICE_PROCESSING:-0}" == "1" ]]; then
      if [[ ! -f "$READINESS_DIR/audio-recording.json" ]]; then
        printf 'readiness failed: audio-recording JSON is missing\n' >&2
        failed=1
      elif ! /usr/bin/grep -Eq '"voiceProcessingEnabled"[[:space:]]*:[[:space:]]*true' "$READINESS_DIR/audio-recording.json"; then
        printf 'readiness failed: audio-recording voice processing was not enabled\n' >&2
        failed=1
      fi
    fi
  fi

  write_readiness_notarization_status

  if [[ -n "${MURMUR_READINESS_TRANSCRIPTION_AUDIO:-}" ]]; then
    run_readiness_command \
      transcription-file \
      --smoke-transcribe-file "$MURMUR_READINESS_TRANSCRIPTION_AUDIO" \
      --smoke-transcribe-model "${MURMUR_READINESS_TRANSCRIPTION_MODEL:-tiny}" \
      --smoke-transcribe-language "${MURMUR_READINESS_TRANSCRIPTION_LANGUAGE:-en}" \
      --smoke-output-json "$READINESS_DIR/transcription-file.json" || failed=1
  fi

  if [[ "${MURMUR_READINESS_MODEL_RUNTIME:-0}" == "1" ]]; then
    run_readiness_command \
      model-runtime \
      --smoke-model-runtime-state "${MURMUR_READINESS_MODEL_RUNTIME_MODEL:-tiny}" \
      --smoke-model-runtime-unload-timeout immediate \
      --smoke-model-runtime-explicit-unload \
      --smoke-output-json "$READINESS_DIR/model-runtime.json" || failed=1
  fi

  if [[ "${MURMUR_READINESS_PERMISSION_SMOKES:-0}" == "1" ]]; then
    run_readiness_command \
      global-shortcut-event-tap \
      --smoke-global-shortcut-event-tap \
      --smoke-global-shortcut-id "${MURMUR_READINESS_SHORTCUT_ID:-readiness}" \
      --smoke-global-shortcut-binding "${MURMUR_READINESS_SHORTCUT_BINDING:-command+control+option+shift+9}" \
      --smoke-output-json "$READINESS_DIR/global-shortcut-event-tap.json" || failed=1
  fi

  if [[ "${MURMUR_READINESS_LIVE_DICTATION_SMOKE:-0}" == "1" ]]; then
    local -a live_dictation_args=(
      --smoke-global-shortcut-recording
      --smoke-global-shortcut-id "${MURMUR_READINESS_LIVE_SHORTCUT_ID:-transcribe}"
      --smoke-global-shortcut-binding "${MURMUR_READINESS_SHORTCUT_BINDING:-command+control+option+shift+9}"
      --smoke-record-duration-ms "${MURMUR_READINESS_RECORD_DURATION_MS:-1500}"
      --smoke-global-shortcut-recording-output "$READINESS_DIR/live-dictation.wav"
      --smoke-transcribe-after-shortcut-recording
      --smoke-transcribe-model "${MURMUR_READINESS_TRANSCRIPTION_MODEL:-tiny}"
      --smoke-transcribe-language "${MURMUR_READINESS_TRANSCRIPTION_LANGUAGE:-en}"
      --smoke-output-json "$READINESS_DIR/live-dictation.json"
    )
    if [[ -n "${MURMUR_READINESS_MICROPHONE:-}" ]]; then
      live_dictation_args+=(--smoke-record-microphone "$MURMUR_READINESS_MICROPHONE")
    fi
    if [[ "${MURMUR_READINESS_SELECTED_SETTINGS:-0}" == "1" ]]; then
      live_dictation_args+=(--smoke-transcribe-selected-settings)
    fi
    if [[ "${MURMUR_READINESS_POST_PROCESS:-0}" == "1" ]]; then
      live_dictation_args+=(--smoke-post-process)
    fi
    if [[ "${MURMUR_READINESS_RECORD_HISTORY:-0}" == "1" ]]; then
      live_dictation_args+=(--smoke-record-history)
    fi
    if [[ "${MURMUR_READINESS_LIVE_PASTE:-0}" == "1" ]]; then
      live_dictation_args+=(
        --smoke-external-paste-after-transcribe
        --smoke-paste-method "${MURMUR_READINESS_PASTE_METHOD:-direct}"
        --smoke-clipboard-handling "${MURMUR_READINESS_CLIPBOARD_HANDLING:-restore}"
      )
    fi

    run_readiness_command live-dictation "${live_dictation_args[@]}" || failed=1
  fi

  if [[ "${MURMUR_READINESS_FOCUS_SMOKES:-0}" == "1" ]]; then
    run_readiness_command \
      external-paste-roundtrip \
      --smoke-external-paste-roundtrip "Murmur native readiness paste" \
      --smoke-paste-method direct \
      --smoke-clipboard-handling restore \
      --smoke-output-json "$READINESS_DIR/external-paste-roundtrip.json" || failed=1
  fi

  if [[ "$failed" -ne 0 ]]; then
    fail "readiness smoke failed; inspect $READINESS_DIR"
  fi

  printf 'readiness output: %s\n' "$READINESS_DIR"
}

case "$MODE" in
  run)
    open_app
    ;;
  --debug|debug)
    lldb -- "$APP_BINARY"
    ;;
  --logs|logs)
    open_app
    /usr/bin/log stream --info --style compact --predicate "process == \"$APP_NAME\""
    ;;
  --telemetry|telemetry)
    open_app
    /usr/bin/log stream --info --style compact --predicate "subsystem == \"$BUNDLE_ID\""
    ;;
  --package|package)
    ;;
  --archive|archive)
    create_archives
    ;;
  --notarize|notarize)
    create_notarized_archives
    ;;
  --release-readiness|release-readiness)
    run_release_readiness
    ;;
  --verify|verify)
    open_app 1 --start-hidden
    wait_for_native
    clean_bundle_xattrs
    /usr/bin/codesign --verify --deep --strict "$APP_BUNDLE" >/dev/null
    if [[ "${MURMUR_KEEP_VERIFY_APP:-0}" != "1" ]]; then
      terminate_existing_native
    fi
    ;;
  --readiness|readiness)
    run_readiness_smokes
    ;;
  *)
    echo "usage: $0 [run|--debug|--logs|--telemetry|--package|--archive|--notarize|--release-readiness|--verify|--readiness]" >&2
    exit 2
    ;;
esac
