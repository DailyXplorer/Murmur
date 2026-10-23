import React from "react";
import ReactDOM from "react-dom/client";
import CallPromptOverlay from "../../src/overlay/CallPromptOverlay";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CallPromptOverlay />
  </React.StrictMode>,
);
