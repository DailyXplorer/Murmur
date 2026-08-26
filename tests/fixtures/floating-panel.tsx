import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import { Dropdown } from "../../src/components/ui/Dropdown";
import "../../src/App.css";

const requestedOptionCount = Number(
  new URLSearchParams(window.location.search).get("options") ?? 12,
);
const optionCount = Number.isFinite(requestedOptionCount)
  ? requestedOptionCount
  : 12;
const options = Array.from({ length: optionCount }, (_, index) => ({
  value: `option-${index + 1}`,
  label: `Option ${index + 1}`,
}));

const FloatingPanelFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>(null);

  return (
    <div className="h-screen bg-background text-text">
      <div data-testid="clipping-container" className="h-72 overflow-hidden">
        <div className="flex h-full items-end justify-center pb-2">
          <Dropdown
            className="w-64"
            options={options}
            selectedValue={selectedValue}
            onSelect={setSelectedValue}
          />
        </div>
      </div>
      <footer
        data-testid="footer"
        className="relative z-[1050] h-24 border-t border-mid-gray/20 bg-background"
      />
    </div>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <FloatingPanelFixture />,
);
