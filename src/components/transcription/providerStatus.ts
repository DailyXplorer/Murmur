import { useCallback, useRef, useState } from "react";
import { commands, type SignInError } from "@/bindings";

export type ProviderStatus =
  | { kind: "checking" }
  | { kind: "configured" }
  | { kind: "unavailable"; reason: "notSignedIn" | "sessionExpired" }
  | { kind: "error" };

/**
 * Reads the local Codex session without claiming a live connection.
 * A request sequence keeps delayed focus checks from replacing newer results.
 */
export const useCodexStatus = () => {
  const [status, setStatus] = useState<ProviderStatus>({ kind: "checking" });
  const requestSequence = useRef(0);

  const refreshStatus = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setStatus({ kind: "checking" });
    let next: ProviderStatus;
    try {
      const auth = await commands.getCodexAuthStatus();
      next = auth.signed_in
        ? { kind: "configured" }
        : {
            kind: "unavailable",
            reason: auth.expired ? "sessionExpired" : "notSignedIn",
          };
    } catch {
      next = { kind: "error" };
    }

    if (sequence !== requestSequence.current) return;
    setStatus(next);
  }, []);

  const invalidatePendingRefreshes = useCallback(() => {
    requestSequence.current += 1;
  }, []);

  return { invalidatePendingRefreshes, refreshStatus, status };
};

export const statusTranslationKey = (status: ProviderStatus): string => {
  switch (status.kind) {
    case "checking":
      return "settings.transcription.checking";
    case "configured":
      return "settings.transcription.chatgpt.signedIn";
    case "unavailable":
      switch (status.reason) {
        case "sessionExpired":
          return "settings.transcription.sessionExpired";
        case "notSignedIn":
          return "settings.transcription.missing";
        default: {
          const _exhaustive: never = status.reason;
          return _exhaustive;
        }
      }
    case "error":
      return "settings.transcription.statusUnavailable";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
};

/** Why a sign-in failed, or `unknown` when the call itself threw. A cancel
 * is not a failure. */
export type SignInFailure = Exclude<SignInError, "cancelled"> | "unknown";

export type SignInState =
  | { phase: "idle" }
  | { phase: "waiting" }
  | { phase: "failed"; error: SignInFailure };

/**
 * Murmur's own ChatGPT sign-in in the browser. `onSignedIn` runs once the
 * session is saved, and the sign-in stays `waiting` until it ends, so the
 * sign-in button does not come back while the new status loads. Leaving the
 * screen does not cancel it: the backend gives up on its own, and a new
 * sign-in replaces the one under way.
 */
export const useChatGptSignIn = (onSignedIn: () => Promise<void>) => {
  const [state, setState] = useState<SignInState>({ phase: "idle" });
  const attempt = useRef(0);

  const start = useCallback(async () => {
    const current = ++attempt.current;
    setState({ phase: "waiting" });
    let next: SignInState = { phase: "idle" };
    try {
      const result = await commands.signInWithChatgpt();
      if (result.status === "ok") {
        await onSignedIn();
      } else if (result.error !== "cancelled") {
        next = { phase: "failed", error: result.error };
      }
    } catch {
      next = { phase: "failed", error: "unknown" };
    }
    if (current === attempt.current) setState(next);
  }, [onSignedIn]);

  const cancel = useCallback(() => {
    attempt.current += 1;
    setState({ phase: "idle" });
    void commands
      .cancelChatgptSignIn()
      .catch((error: unknown) =>
        console.warn("Failed to cancel the ChatGPT sign-in:", error),
      );
  }, []);

  return { cancel, signIn: state, start };
};

export const signInErrorKey = (error: SignInFailure): string => {
  switch (error) {
    case "port_busy":
      return "settings.transcription.chatgpt.errors.portBusy";
    case "timed_out":
      return "settings.transcription.chatgpt.errors.timedOut";
    case "denied":
      return "settings.transcription.chatgpt.errors.denied";
    case "not_entitled":
      return "settings.transcription.chatgpt.errors.notEntitled";
    case "network":
      return "settings.transcription.chatgpt.errors.network";
    case "rejected":
      return "settings.transcription.chatgpt.errors.rejected";
    case "save_failed":
      return "settings.transcription.chatgpt.errors.saveFailed";
    case "browser_failed":
      return "settings.transcription.chatgpt.errors.browserFailed";
    case "unknown":
      return "settings.transcription.chatgpt.errors.generic";
    default: {
      const _exhaustive: never = error;
      return _exhaustive;
    }
  }
};
