import React from "react";
import ReactDOM from "react-dom/client";
import RecordingOverlay from "../../src/overlay/RecordingOverlay";

const root = ReactDOM.createRoot(
  document.getElementById("root") as HTMLElement,
);
const meetingLayout = new URLSearchParams(window.location.search).get(
  "meetingLayout",
);

root.render(
  <React.StrictMode>
    <RecordingOverlay
      meetingLayout={meetingLayout === "inline" ? "inline" : "stacked"}
    />
  </React.StrictMode>,
);

Object.assign(window, {
  unmountRecordingOverlay: () => root.unmount(),
});
