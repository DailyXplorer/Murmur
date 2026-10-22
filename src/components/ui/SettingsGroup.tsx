import React from "react";

interface SettingsGroupProps {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}

export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  action,
  children,
}) => {
  return (
    <section className="space-y-2">
      <div className="flex min-h-6 items-center justify-between gap-4 px-4">
        <h2 className="min-w-0 text-balance font-title text-base font-normal leading-6 text-text">
          {title}
        </h2>
        {action && <div className="shrink-0">{action}</div>}
      </div>
      <div className="overflow-visible rounded-lg border border-mid-gray/20 bg-background">
        <div className="divide-y divide-mid-gray/20">{children}</div>
      </div>
    </section>
  );
};
