import React from "react";
import ReactDOM from "react-dom/client";
import { ToggleSwitch } from "../../src/components/ui/ToggleSwitch";
import "../../src/App.css";

const ToggleSwitchFixture: React.FC = () => (
  <main className="flex h-screen items-center justify-center bg-background text-text">
    <div className="w-96">
      <ToggleSwitch
        checked
        onChange={() => undefined}
        label="Show what's new"
        description="Show release notes after updates"
        descriptionMode="inline"
      />
    </div>
  </main>
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <ToggleSwitchFixture />,
);
