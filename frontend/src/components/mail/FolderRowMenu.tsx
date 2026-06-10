"use client";

import { MoreHorizontal } from "lucide-react";
import type { Folder } from "@/types/folder";
import { isSystemFolder } from "./FolderContextMenu";

type FolderRowMenuProps = {
  folder: Folder;
  onOpen: (pos: { x: number; y: number }) => void;
};

export function FolderRowMenu({ folder, onOpen }: FolderRowMenuProps) {
  if (isSystemFolder(folder.name)) return null;

  return (
    <button
      type="button"
      aria-label="Folder options"
      onClick={(e) => {
        e.stopPropagation();
        const rect = e.currentTarget.getBoundingClientRect();
        // Anchor to right edge of button so the menu doesn't overflow on mobile
        onOpen({ x: rect.right, y: rect.bottom });
      }}
      className="md:hidden flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
    >
      <MoreHorizontal className="size-3.5" />
    </button>
  );
}
