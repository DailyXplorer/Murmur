import React from "react";

interface SettingsGroupProps {
  title: string;
  description?: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}

export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  action,
  children,
}) => {
  return (
    <section className="space-y-2">
      <div className="flex min-h-6 items-center justify-between gap-4 px-4">
        <div className="min-w-0">
          <h2 className="text-balance text-xs font-medium uppercase tracking-wide text-mid-gray">
            {title}
          </h2>
          {description && (
            <p className="mt-1 text-pretty text-xs text-mid-gray">
              {description}
            </p>
          )}
        </div>
        {action && <div className="shrink-0">{action}</div>}
      </div>
      <div className="overflow-visible rounded-lg border border-mid-gray/20 bg-background">
        <div className="divide-y divide-mid-gray/20">{children}</div>
      </div>
    </section>
  );
};
