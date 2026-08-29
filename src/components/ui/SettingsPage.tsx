import React from "react";

interface SettingsPageProps {
  label: string;
  children: React.ReactNode;
}

export const SettingsPage: React.FC<SettingsPageProps> = ({
  label,
  children,
}) => (
  <main aria-label={label} className="mx-auto w-full max-w-3xl space-y-6 pb-1">
    <h1 className="sr-only">{label}</h1>
    {children}
  </main>
);
