import React from "react";
import ReactDOM from "react-dom/client";
import { AudioPlayer } from "../../src/components/ui/AudioPlayer";
import { Button } from "../../src/components/ui/Button";
import { Slider } from "../../src/components/ui/Slider";
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
      <Slider
        value={0.5}
        onChange={() => undefined}
        min={0}
        max={1}
        label="Overlay size"
        description="Adjust the recording overlay size"
        showValue={false}
      />
      <AudioPlayer />
    </div>
  </main>
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <AccentControlsFixture />,
);
