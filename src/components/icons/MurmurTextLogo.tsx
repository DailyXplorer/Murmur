import React from "react";
import murmurTextLogo from "@/assets/murmur-text-logo.png";

const MurmurTextLogo = ({
  width,
  height,
  className,
}: {
  width?: number;
  height?: number;
  className?: string;
}) => {
  return (
    <img
      src={murmurTextLogo}
      alt="Murmur"
      width={width}
      height={height}
      className={`accent-logo ${className ?? ""}`}
      draggable={false}
    />
  );
};

export default MurmurTextLogo;
