// What Tauri's OS plugin injects into every Murmur window on macOS. Import
// it first: `@/lib/platform` reads it as soon as it loads.
Object.assign(window, {
  __TAURI_OS_PLUGIN_INTERNALS__: {
    arch: "aarch64",
    eol: "\n",
    exe_extension: "",
    family: "unix",
    os_type: "macos",
    platform: "macos",
    version: "15.0.0",
  },
});
