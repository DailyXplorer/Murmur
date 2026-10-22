import React from "react";
import { FIELD_SIZE_CLASSES, FIELD_SURFACE_CLASSES } from "./controlClasses";

export const Input = React.forwardRef<
  HTMLInputElement,
  React.InputHTMLAttributes<HTMLInputElement>
>(({ className = "", disabled, ...props }, ref) => {
  const interactiveClasses = disabled
    ? "opacity-60 cursor-not-allowed bg-mid-gray/10 border-mid-gray/40"
    : `${FIELD_SURFACE_CLASSES} hover:bg-logo-primary/10 hover:border-logo-primary focus:outline-none focus:bg-logo-primary/20 focus:border-accent-text`;

  return (
    <input
      ref={ref}
      data-control="field"
      className={`${FIELD_SIZE_CLASSES} transition-all duration-150 ${interactiveClasses} ${className}`}
      disabled={disabled}
      {...props}
    />
  );
});
Input.displayName = "Input";
