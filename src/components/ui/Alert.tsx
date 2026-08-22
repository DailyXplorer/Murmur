import React from "react";
import { CheckCircleIcon } from "@phosphor-icons/react/dist/csr/CheckCircle";
import { InfoIcon } from "@phosphor-icons/react/dist/csr/Info";
import { WarningCircleIcon } from "@phosphor-icons/react/dist/csr/WarningCircle";
import { WarningIcon } from "@phosphor-icons/react/dist/csr/Warning";
import type { Icon } from "@phosphor-icons/react/dist/lib/types";

type AlertVariant = "error" | "warning" | "info" | "success";

interface AlertProps {
  variant?: AlertVariant;
  /** When true, removes rounded corners for use inside containers */
  contained?: boolean;
  children: React.ReactNode;
  className?: string;
}

const variantStyles: Record<
  AlertVariant,
  { container: string; icon: string; text: string }
> = {
  error: {
    container: "bg-red-500/10",
    icon: "text-red-500",
    text: "text-red-400",
  },
  warning: {
    container: "bg-yellow-500/10",
    icon: "text-yellow-500",
    text: "text-yellow-400",
  },
  info: {
    container: "bg-blue-500/10",
    icon: "text-blue-500",
    text: "text-blue-400",
  },
  success: {
    container: "bg-green-500/10",
    icon: "text-green-500",
    text: "text-green-400",
  },
};

const variantIcons: Record<AlertVariant, Icon> = {
  error: WarningCircleIcon,
  warning: WarningIcon,
  info: InfoIcon,
  success: CheckCircleIcon,
};

export const Alert: React.FC<AlertProps> = ({
  variant = "error",
  contained = false,
  children,
  className = "",
}) => {
  const styles = variantStyles[variant];
  const Icon = variantIcons[variant];

  return (
    <div
      className={`flex items-start gap-3 p-4 ${styles.container} ${contained ? "" : "rounded-lg"} ${className}`}
    >
      <Icon size={18} className={`shrink-0 mt-0.5 ${styles.icon}`} />
      <p className={`text-sm ${styles.text}`}>{children}</p>
    </div>
  );
};
