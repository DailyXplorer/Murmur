import { fitCard } from "./fitText";
import { SHARE_SIZE } from "./shareStats";

const readBase64 = (blob: Blob): Promise<string> =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Read failed"));
    reader.onload = () => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.readAsDataURL(blob);
  });

/**
 * Draws the share card as a 1080 x 1080 PNG, whatever the screen's pixel
 * ratio, and returns it as base64 for the app to copy or save. Nothing
 * leaves the webview but the picture, and the picture is built from the
 * card's numbers only.
 */
export const renderShareCard = async (card: HTMLElement): Promise<string> => {
  const { toCanvas } = await import("html-to-image");
  // The fonts must be in before the card is measured and drawn.
  await document.fonts.ready;
  fitCard(card);
  const options = {
    width: SHARE_SIZE,
    height: SHARE_SIZE,
    pixelRatio: 1,
    backgroundColor: "#fbfbfb",
  };
  // WebKit (the Mac's webview) can paint the images of an SVG drawing blank
  // the first time it decodes them, so the first pass only warms it up.
  await toCanvas(card, options);
  const canvas = await toCanvas(card, options);
  const blob = await new Promise<Blob | null>((resolve) =>
    canvas.toBlob(resolve, "image/png"),
  );
  if (!blob) throw new Error("The card could not be drawn");
  return readBase64(blob);
};
