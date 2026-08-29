import React from "react";

interface SettingControlGroupProps {
  primary: React.ReactNode;
  action: React.ReactNode;
  className?: string;
}

export const SettingControlGroup: React.FC<SettingControlGroupProps> = ({
  primary,
  action,
  className = "",
}) => (
  <div
    data-slot="control-group"
    className={`grid max-w-full grid-cols-[var(--settings-control-rail-width)_max-content] items-center gap-2 ${className}`}
  >
    <div data-slot="control-primary" className="min-w-0">
      {primary}
    </div>
    <div
      data-slot="control-action"
      className="flex shrink-0 items-center justify-end"
    >
      {action}
    </div>
  </div>
);
