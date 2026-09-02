import type {
  CodexAuthStatus,
  GeminiStatus,
  MetaApiStatus,
  MetaAppStatus,
} from "@/bindings";

type TranscriptionServiceReadiness = {
  codex: Pick<CodexAuthStatus, "signed_in">;
  gemini: Pick<GeminiStatus, "installed" | "signed_in"> | null;
  meta: Pick<MetaApiStatus, "configured"> | null;
  metaApp: Pick<MetaAppStatus, "ready"> | null;
};

export const hasUsableTranscriptionService = ({
  codex,
  gemini,
  meta,
  metaApp,
}: TranscriptionServiceReadiness): boolean =>
  codex.signed_in ||
  Boolean(gemini?.installed && gemini.signed_in) ||
  Boolean(meta?.configured) ||
  Boolean(metaApp?.ready);
