import { openUrl } from "@tauri-apps/plugin-opener";

const PROJECT_URL = "https://github.com/DailyXplorer/Murmur";

export const openProjectPage = async () => {
  try {
    await openUrl(PROJECT_URL);
  } catch (error) {
    console.error("Failed to open the project page:", error);
  }
};
