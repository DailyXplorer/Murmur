import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** Every text button is filled; `danger` marks destructive actions. Icon-only
   * actions use `IconButton`. */
  variant?: "primary" | "danger";
  size?: "text" | "chip";
}

const SIZE_CLASSES = {
  text: "h-control min-w-control-button px-4",
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
    danger:
      "text-red-700 dark:text-red-300 bg-red-500/15 border-transparent hover:bg-red-500/25 active:bg-red-500/30 focus-visible:ring-red-500",
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
