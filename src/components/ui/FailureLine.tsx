import React from "react";

interface FailureLineProps {
  message: string;
  /** The way out, such as a Retry button, on the right. */
  action?: React.ReactNode;
  /** Padding that lines the message up with its page's rows. */
  className?: string;
}

/** A failure told as one muted line. */
export const FailureLine: React.FC<FailureLineProps> = ({
  message,
  action,
  className = "",
}) => (
  <div role="alert" className={`flex items-center gap-3 ${className}`}>
    <p className="flex-1 text-sm text-mid-gray">{message}</p>
    {action}
  </div>
);
