import React from "react";

interface ProgressBarProps {
  percentage: number;
  className?: string;
  size?: "small" | "medium" | "large";
}

const ProgressBar: React.FC<ProgressBarProps> = ({
  percentage,
  className = "",
  size = "medium",
}) => {
  const sizeClasses = {
    small: "w-16 h-1",
    medium: "w-20 h-1.5",
    large: "w-24 h-2",
  };

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <progress
        value={Math.max(0, Math.min(100, percentage))}
        max={100}
        className={`${sizeClasses[size]} [&::-webkit-progress-bar]:rounded-full [&::-webkit-progress-bar]:bg-mid-gray/20 [&::-webkit-progress-value]:rounded-full [&::-webkit-progress-value]:bg-logo-primary`}
      />
    </div>
  );
};

export default ProgressBar;
