import React from "react";
import ReactDOM from "react-dom/client";
import { Button } from "../../src/components/ui/Button";
import { ToggleSwitch } from "../../src/components/ui/ToggleSwitch";
import "../../src/App.css";

const AccentControlsFixture: React.FC = () => (
  <main className="flex h-screen items-center justify-center bg-background text-text">
    <div className="flex w-96 flex-col gap-8">
      <ToggleSwitch
        checked
        onChange={() => undefined}
        label="Show what's new"
        description="Show release notes after updates"
        descriptionMode="inline"
      />
      <Button>Donate</Button>
    </div>
  </main>
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <AccentControlsFixture />,
);
