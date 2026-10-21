import { readFileSync } from "node:fs";

const libRs = readFileSync(
  new URL("../src-tauri/src/lib.rs", import.meta.url),
  "utf8",
);

const windowSize = (name: string) => {
  const match = libRs.match(
    new RegExp(`const ${name}: \\(f64, f64\\) = \\(([\\d.]+), ([\\d.]+)\\);`),
  );
  if (!match) throw new Error(`${name} not found in src-tauri/src/lib.rs`);
  return { width: Number(match[1]), height: Number(match[2]) };
};

/** The size the settings window opens at on a screen large enough for it. */
export const DEFAULT_WINDOW = windowSize("SETTINGS_WINDOW_SIZE");
/** The smallest size the settings window can be made. */
export const MIN_WINDOW = windowSize("SETTINGS_WINDOW_MIN_SIZE");
