"use client";

import { useState, useRef, useEffect, useCallback, type ReactNode } from "react";
import { Pencil, Trash2 } from "lucide-react";
import { useDeleteFolder } from "@/hooks/useFolders";
import { useLongPress } from "@/hooks/useLongPress";
import { cn } from "@/lib/utils";

/** System folders that cannot be renamed or deleted. */
const SYSTEM_FOLDERS = new Set([
  "INBOX",
  "Sent",
  "Drafts",
  "Trash",
  "Junk",
  "Spam",
]);

export function isSystemFolder(name: string): boolean {
  return SYSTEM_FOLDERS.has(name);
}

interface FolderContextMenuProps {
  folderName: string;
  onRename: () => void;
  children: ReactNode;
  onDragOver?: (e: React.DragEvent) => void;
  onDragEnter?: (e: React.DragEvent) => void;
  onDragLeave?: (e: React.DragEvent) => void;
  onDrop?: (e: React.DragEvent) => void;
  /** Exposed so external callers (FolderRowMenu) can open the menu programmatically. */
  onOpenMenu?: (handler: (pos: { x: number; y: number }) => void) => void;
}

export function FolderContextMenu({
  folderName,
  onRename,
  children,
  onDragOver,
  onDragEnter,
  onDragLeave,
  onDrop,
  onOpenMenu,
}: FolderContextMenuProps) {
  const [menuPos, setMenuPos] = useState<{ x: number; y: number } | null>(
    null,
  );
  const [confirmDelete, setConfirmDelete] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const deleteFolder = useDeleteFolder();

  const isSystem = isSystemFolder(folderName);

  const closeMenu = useCallback(() => {
    setMenuPos(null);
    setConfirmDelete(false);
  }, []);

  const openFolderMenu = useCallback(
    (pos: { x: number; y: number }) => {
      if (isSystem) return;
      setMenuPos(pos);
      setConfirmDelete(false);
    },
    [isSystem],
  );

  // Expose openFolderMenu to parent via callback ref pattern
  useEffect(() => {
    onOpenMenu?.(openFolderMenu);
  }, [onOpenMenu, openFolderMenu]);

  const handleContextMenu = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      openFolderMenu({ x: e.clientX, y: e.clientY });
    },
    [openFolderMenu],
  );

  const longPress = useLongPress({
    onLongPress: (e) => {
      const src = "touches" in e ? e.touches[0] : e;
      openFolderMenu({ x: src.clientX, y: src.clientY });
    },
  });

  // Close on click outside
  useEffect(() => {
    if (!menuPos) return;
    function handleClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        closeMenu();
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [menuPos, closeMenu]);

  // Close on Escape
  useEffect(() => {
    if (!menuPos) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        closeMenu();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [menuPos, closeMenu]);

  const handleRename = useCallback(() => {
    closeMenu();
    onRename();
  }, [closeMenu, onRename]);

  const handleDelete = useCallback(() => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    deleteFolder.mutate(
      { name: folderName },
      { onSuccess: () => closeMenu() },
    );
  }, [confirmDelete, deleteFolder, folderName, closeMenu]);

  return (
    <div
      onContextMenu={handleContextMenu}
      onDragOver={onDragOver}
      onDragEnter={onDragEnter}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      {...longPress}
      style={{ userSelect: "none", WebkitUserSelect: "none" }}
    >
      {children}

      {menuPos && (
        <div
          ref={menuRef}
          className={cn(
            "fixed z-50 min-w-[160px] rounded-md border border-border bg-popover py-1 shadow-md",
          )}
          style={{
            left: Math.max(4, Math.min(menuPos.x, window.innerWidth - 204)),
            top: Math.max(4, Math.min(menuPos.y, window.innerHeight - 88)),
          }}
        >
          <button
            type="button"
            onClick={handleRename}
            className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent"
          >
            <Pencil className="size-3.5" />
            Rename
          </button>
          <button
            type="button"
            onClick={handleDelete}
            disabled={deleteFolder.isPending}
            className={cn(
              "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent",
              confirmDelete
                ? "text-destructive font-medium"
                : "text-foreground",
            )}
          >
            <Trash2 className="size-3.5" />
            {deleteFolder.isPending
              ? "Deleting..."
              : confirmDelete
                ? "Confirm delete?"
                : "Delete"}
          </button>
        </div>
      )}
    </div>
  );
}
