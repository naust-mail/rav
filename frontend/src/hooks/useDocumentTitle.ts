"use client";

import { useEffect } from "react";
import { useFolders } from "@/hooks/useFolders";

/** Updates the browser tab title to show inbox unread count when non-zero. */
export function useDocumentTitle() {
  const { data } = useFolders();

  useEffect(() => {
    const inbox = data?.folders.find(
      (f) => f.name === "INBOX" || f.name.toLowerCase() === "inbox",
    );
    const unread = inbox?.unread_count ?? 0;
    document.title = unread > 0 ? `(${unread}) oxi.email` : "oxi.email";
  }, [data]);
}
