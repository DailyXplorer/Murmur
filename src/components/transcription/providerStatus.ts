import { useCallback, useRef, useState } from "react";
import { commands } from "@/bindings";

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
      return "onboarding.provider.configured";
    case "unavailable":
      switch (status.reason) {
        case "sessionExpired":
          return "settings.transcription.sessionExpired";
        case "notSignedIn":
          return "onboarding.providerLabels.unavailable";
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
