import { params } from "./params";

/** `account=none`: no ChatGPT session on this Mac; `account=expired`: one
 * that can no longer be refreshed. */
let account = params.get("account") ?? "signedIn";
/** `signOut=fails`: the backend cannot remove the session. */
const signOutFails = params.get("signOut") === "fails";
/** The browser sign-in under way, as the backend runs it. */
let signIn: {
  resolve: (value: null) => void;
  reject: (error: string) => void;
} | null = null;
/** The ChatGPT session commands, which change the account. */
export const handleAccountCommand = (
  command: string,
): { handled: boolean; value?: unknown } => {
  switch (command) {
    case "get_codex_auth_status":
      return {
        handled: true,
        value: {
          expired: account === "expired",
          signed_in: account === "signedIn",
        },
      };
    case "sign_in_with_chatgpt":
      signIn?.reject("cancelled");
      return {
        handled: true,
        value: new Promise((resolve, reject) => {
          signIn = { resolve, reject };
        }),
      };
    case "cancel_chatgpt_sign_in":
      signIn?.reject("cancelled");
      signIn = null;
      return { handled: true, value: null };
    case "sign_out_of_chatgpt":
      if (signOutFails) {
        return {
          handled: true,
          value: Promise.reject("Couldn't remove auth.json"),
        };
      }
      account = "none";
      return { handled: true, value: null };
    default:
      return { handled: false };
  }
};

export const resolveSignIn = () => {
  account = "signedIn";
  signIn?.resolve(null);
  signIn = null;
};
