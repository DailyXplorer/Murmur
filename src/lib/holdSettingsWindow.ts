import { commands } from "@/bindings";

/**
 * Runs `work` while closing the settings window only hides it, for work
 * whose later steps live in this page and would die with a destroyed window.
 */
export async function holdSettingsWindow<T>(work: () => Promise<T>) {
  await commands.holdSettingsWindow();
  try {
    return await work();
  } finally {
    void commands.releaseSettingsWindow();
  }
}
