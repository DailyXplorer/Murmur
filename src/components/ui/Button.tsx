import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "primary-soft"
    | "secondary"
    | "warning"
    | "danger"
    | "danger-ghost"
    | "ghost";
  size?: "text" | "icon" | "chip";
}

const SIZE_CLASSES = {
  text: "h-control min-w-control-button px-4",
  icon: "size-control shrink-0",
  chip: "h-control px-3",
} as const;

export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  variant = "primary",
  size = "text",
  ...props
}) => {
  const baseClasses =
    "inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium rounded-md border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:ring-offset-2 focus-visible:ring-offset-background transition-[color,background-color,border-color,transform] active:scale-[0.96] disabled:opacity-50 disabled:cursor-not-allowed disabled:active:scale-100 cursor-pointer";

  const variantClasses = {
    primary:
      "text-on-accent bg-background-ui border-background-ui hover:bg-background-ui-hover hover:border-background-ui-hover active:bg-background-ui-active active:border-background-ui-active focus:ring-2 focus:ring-text focus:ring-offset-2 focus:ring-offset-background",
    "primary-soft":
      "text-text bg-logo-primary/20 border-transparent hover:bg-logo-primary/30",
    secondary:
      "bg-mid-gray/10 border-mid-gray/20 hover:bg-background-ui/30 hover:border-logo-primary",
    warning:
      "text-text bg-mid-gray/10 border-mid-gray/20 hover:bg-warning/15 hover:border-warning focus-visible:ring-warning",
    danger:
      "text-white bg-red-600 border-mid-gray/20 hover:bg-red-700 hover:border-red-700 focus-visible:ring-red-500",
    "danger-ghost":
      "text-red-400 border-transparent hover:text-red-300 hover:bg-red-500/10 focus-visible:bg-red-500/20 focus-visible:ring-red-500",
    ghost:
      "text-current border-transparent hover:bg-mid-gray/10 hover:border-logo-primary focus-visible:bg-mid-gray/20",
  };

  return (
    <button
      data-control={size}
      className={`${baseClasses} ${variantClasses[variant]} ${SIZE_CLASSES[size]} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
};
