import React from "react";
import ReactDOM from "react-dom/client";
import RecordingOverlay from "../../src/overlay/RecordingOverlay";

const root = ReactDOM.createRoot(
  document.getElementById("root") as HTMLElement,
);

root.render(
  <React.StrictMode>
    <RecordingOverlay />
  </React.StrictMode>,
);

Object.assign(window, {
  unmountRecordingOverlay: () => root.unmount(),
});
