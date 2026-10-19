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
  // Suspend and resume run off the main thread, so a quick begin then end
  // could reach the backend reversed and leave every shortcut off: each
  // call waits for the previous one.
  let queue: Promise<unknown> = Promise.resolve();
  const send = (call: () => Promise<unknown>) => {
    queue = queue.then(call).catch(console.error);
  };

  return {
    capturingId: null,
    begin: (id) => {
      if (get().capturingId === null) {
        window.addEventListener("blur", abandon);
        document.addEventListener("visibilitychange", abandonWhenHidden);
        send(commands.suspendAllBindings);
      }
      set({ capturingId: id });
    },
    end: (id) => {
      if (get().capturingId !== id) return;
      window.removeEventListener("blur", abandon);
      document.removeEventListener("visibilitychange", abandonWhenHidden);
      set({ capturingId: null });
      send(commands.resumeAllBindings);
    },
  };
});
