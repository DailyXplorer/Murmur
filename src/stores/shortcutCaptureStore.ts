import { create } from "zustand";
import { commands } from "@/bindings";

interface ShortcutCaptureStore {
  /** The one shortcut field recording keys, if any. */
  capturingId: string | null;
  begin: (id: string) => void;
  /** Ends the capture if `id` still owns it. */
  end: (id: string) => void;
}

/**
 * Global shortcuts stay suspended exactly while a field captures keys.
 * Starting a capture in another field moves it there, so one keypress can
 * never rebind two actions. Leaving the window ends the capture, like a
 * native recorder that stops when its window resigns key: otherwise
 * switching apps or closing Settings would leave every shortcut off.
 */
export const useShortcutCaptureStore = create<ShortcutCaptureStore>()((
  set,
  get,
) => {
  const abandon = () => {
    const { capturingId, end } = get();
    if (capturingId !== null) end(capturingId);
  };
  const abandonWhenHidden = () => {
    if (document.visibilityState === "hidden") abandon();
  };

  return {
    capturingId: null,
    begin: (id) => {
      if (get().capturingId === null) {
        window.addEventListener("blur", abandon);
        document.addEventListener("visibilitychange", abandonWhenHidden);
        void commands.suspendAllBindings().catch(console.error);
      }
      set({ capturingId: id });
    },
    end: (id) => {
      if (get().capturingId !== id) return;
      window.removeEventListener("blur", abandon);
      document.removeEventListener("visibilitychange", abandonWhenHidden);
      set({ capturingId: null });
      void commands.resumeAllBindings().catch(console.error);
    },
  };
});
