"use client";

import { useSyncExternalStore } from "react";

const MOBILE_BREAKPOINT = 768;

export function useIsMobile(): boolean {
  const subscribe = (callback: () => void) => {
    const mq = window.matchMedia(
        `(max-width: ${MOBILE_BREAKPOINT - 1}px)`
    );

    mq.addEventListener("change", callback);

    return () => mq.removeEventListener("change", callback);
  };

  const getSnapshot = () =>
      window.matchMedia(
          `(max-width: ${MOBILE_BREAKPOINT - 1}px)`
      ).matches;

  const getServerSnapshot = () => false;

  return useSyncExternalStore(
      subscribe,
      getSnapshot,
      getServerSnapshot
  );
}