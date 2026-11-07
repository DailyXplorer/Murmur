/**
 * Shrinks the text of the share card that would not fit its slot, so a huge
 * number, a long translation or a wide font never runs out of the card.
 *
 * An element marked `data-fit="<group>"` and `data-fit-max="<px>"` is a
 * single line that starts at its largest size and must fit the content width
 * of its parent. The elements of a group share one size, the smallest any
 * of them needs, so a pair of tiles stays level. Sizes are set on the
 * elements themselves, in pixels: the export copies computed styles, so what
 * the preview shows is what the picture holds. Widths are layout widths
 * (`offsetWidth`), which a scaled preview does not change.
 */

/** Never smaller than this share of the largest size. */
const MIN_SHARE = 0.25;
const MAX_PASSES = 8;

const contentWidth = (element: HTMLElement): number => {
  const style = getComputedStyle(element);
  return (
    element.clientWidth -
    (parseFloat(style.paddingLeft) || 0) -
    (parseFloat(style.paddingRight) || 0)
  );
};

/** How much of its room the element takes at the current size. */
const take = (element: HTMLElement): number => {
  const room = element.parentElement ? contentWidth(element.parentElement) : 0;
  return room > 0 ? element.offsetWidth / room : 1;
};

export const fitCard = (root: HTMLElement): void => {
  const groups = new Map<string, HTMLElement[]>();
  for (const element of root.querySelectorAll<HTMLElement>("[data-fit]")) {
    const group = element.dataset.fit ?? "";
    groups.set(group, [...(groups.get(group) ?? []), element]);
  }

  for (const elements of groups.values()) {
    const maxima = elements.map((element) => Number(element.dataset.fitMax));
    const apply = (share: number) =>
      elements.forEach((element, i) => {
        element.style.fontSize = `${(maxima[i] * share).toFixed(2)}px`;
      });

    let share = 1;
    apply(share);
    // Text does not narrow in exact proportion to its size (glyphs are
    // rounded), so each size is measured after it is applied.
    for (let pass = 0; pass < MAX_PASSES; pass++) {
      const widest = Math.max(...elements.map(take));
      if (widest <= 1) break;
      share = Math.max(MIN_SHARE, (share / widest) * 0.995);
      apply(share);
      if (share === MIN_SHARE) break;
    }
  }
};
