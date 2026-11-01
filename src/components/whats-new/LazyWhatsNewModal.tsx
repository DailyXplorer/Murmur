import React, { lazy, Suspense } from "react";

const WhatsNewModal = lazy(() =>
  import("./WhatsNewModal").then((module) => ({
    default: module.WhatsNewModal,
  })),
);

/** The modal and its Markdown renderer load with the first note shown. */
export const LazyWhatsNewModal: React.FC<
  React.ComponentProps<typeof WhatsNewModal>
> = (props) => (
  <Suspense fallback={null}>
    <WhatsNewModal {...props} />
  </Suspense>
);
