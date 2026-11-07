import murmurTextLogo from "@/assets/murmur-text-logo.webp";

// The file is the original wordmark at 2x its largest display size (400 px
// wide); the ratio below is the original's, so the layout does not move.
const LOGO_ASPECT_RATIO = "1217 / 358";

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
      style={{ aspectRatio: LOGO_ASPECT_RATIO }}
      draggable={false}
    />
  );
};

export default MurmurTextLogo;
