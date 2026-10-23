// Harnesses that Playwright installs on `window` inside the page. The
// fixtures that own a harness declare it themselves; these are the ones
// that exist only inside `page.evaluate` / `addInitScript`.
export {};

declare global {
  interface Window {
    __TAURI_INTERNALS__?: {
      invoke(command: string, args?: unknown): Promise<unknown>;
      transformCallback?(callback: (event: unknown) => unknown): number;
      unregisterCallback?(id: number): void;
      runCallback?(id: number, event: unknown): boolean;
    };
    __TAURI_EVENT_PLUGIN_INTERNALS__?: {
      unregisterListener(event: string, id: number): void;
    };
    recordingOverlayListenerHarness: {
      pendingCount(): number;
      resolveNext(): boolean;
      settleUntilIdle(): Promise<void>;
      dispatch(event: string, payload?: unknown): number;
      dispatchById(id: number, event: string, payload?: unknown): boolean;
      summary(): {
        rejectedListener: boolean;
        successfulListenerIds: number[];
        unlistenCounts: Array<[number, number]>;
        listenerAttempts: string[];
        registeredListeners: Array<[number, string]>;
      };
    };
    unmountRecordingOverlay(): void;
    overlayHarness: {
      dispatch(event: string, payload?: unknown): number;
      releaseSettings(): void;
      copyCalls(): number;
      /** Capsule sizes the copy pill asked its window to fit, oldest first. */
      copyFits(): Array<{ width: number; height: number }>;
      /** The call prompt's answers, oldest first. */
      callAnswers(): Array<{ prompt: string; accept: boolean }>;
    };
  }
}
