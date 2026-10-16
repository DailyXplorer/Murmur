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
 * never rebind two actions.
 */
export const useShortcutCaptureStore = create<ShortcutCaptureStore>()(
  (set, get) => ({
    capturingId: null,
    begin: (id) => {
      if (get().capturingId === null) {
        void commands.suspendAllBindings().catch(console.error);
      }
      set({ capturingId: id });
    },
    end: (id) => {
      if (get().capturingId !== id) return;
      set({ capturingId: null });
      void commands.resumeAllBindings().catch(console.error);
    },
  }),
);
