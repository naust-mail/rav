"use client";

import { useState, useRef, useEffect, useCallback, useMemo } from "react";
import {
  Inbox,
  Send,
  FileText,
  Trash2,
  AlertCircle,
  Star,
  Folder,
  Loader2,
  FolderPlus,
  Check,
  X,
  ChevronRight,
  Settings,
} from "lucide-react";
import { useIsFetching } from "@tanstack/react-query";
import { useIsMobile } from "@/hooks/useIsMobile";
import { useFolders, useRenameFolder } from "@/hooks/useFolders";
import { useQuota } from "@/hooks/useQuota";
import { useMoveMessage } from "@/hooks/useMessages";
import { useUiStore } from "@/stores/useUiStore";
import { Button } from "@/components/ui/button";
import { FolderContextMenu } from "@/components/mail/FolderContextMenu";
import { FolderRowMenu } from "@/components/mail/FolderRowMenu";
import { CreateFolderDialog } from "@/components/mail/CreateFolderDialog";
import { AccountSwitcher } from "@/components/accounts/AccountSwitcher";
import { cn } from "@/lib/utils";
import { TagSection } from "@/components/mail/TagSection";
import type { Folder as FolderType } from "@/types/folder";

// ---------------------------------------------------------------------------
// Public helpers (used by MessageList and other components)
// ---------------------------------------------------------------------------

/** Map raw IMAP folder names to user-friendly display names. */
export function formatFolderName(name: string): string {
  const lower = name.toLowerCase();
  if (lower === "inbox") return "Inbox";
  if (lower === "drafts" || lower === "[gmail]/drafts") return "Drafts";
  if (lower === "sent" || lower === "[gmail]/sent mail") return "Sent";
  if (lower === "trash" || lower === "[gmail]/trash") return "Trash";
  if (lower === "junk" || lower === "spam" || lower === "[gmail]/spam") return "Spam";
  if (lower === "archive" || lower === "[gmail]/all mail") return "Archive";
  if (lower === "starred" || lower === "flagged") return "Starred";
  return name;
}

/** Returns true when the folder name refers to the Drafts mailbox. */
export function isDraftsFolder(name: string): boolean {
  const lower = name.toLowerCase();
  return lower === "drafts" || lower === "[gmail]/drafts";
}

// ---------------------------------------------------------------------------
// Folder tree builder
// ---------------------------------------------------------------------------

/** Sort priority for well-known folders — lower value sorts higher. */
function folderSortOrder(folder: FolderType, displayName: string): number {
  if (folder.name.toLowerCase() === "inbox") return 0;
  // Use IMAP special-use attributes first — more reliable than name matching.
  const attrs = folder.attributes.map((a) => a.toLowerCase());
  if (attrs.includes("\\drafts")) return 1;
  if (attrs.includes("\\sent")) return 2;
  if (attrs.includes("\\junk")) return 3;
  if (attrs.includes("\\trash")) return 4;
  if (attrs.includes("\\archive") || attrs.includes("\\all")) return 5;
  // Name-based fallback for servers that don't set special-use attributes.
  const lower = displayName.toLowerCase();
  if (lower === "drafts" || lower === "draft") return 1;
  if (lower === "sent" || lower === "sent mail" || lower === "sent items") return 2;
  if (lower === "junk" || lower === "spam") return 3;
  if (lower === "trash" || lower === "deleted" || lower === "deleted items") return 4;
  if (lower === "archive" || lower === "all mail") return 5;
  return 6;
}

type TreeNode = {
  folder: FolderType;
  /** Last path segment shown in the sidebar (e.g. "Work" for "Projects/Work"). */
  displayName: string;
  /**
   * Parent path prefix *including* the trailing delimiter (e.g. "Projects/").
   * Empty string for top-level folders.
   */
  parentPath: string;
  /** 0-based nesting depth. */
  depth: number;
  children: TreeNode[];
};

/**
 * Converts a flat Folder[] (as returned by GET /api/folders) into a sorted tree.
 *
 * The tree respects the IMAP delimiter character (usually "/") stored on each
 * folder.  Folders without a matching parent in the list are treated as roots —
 * this handles edge cases where Dovecot doesn't emit a \Noselect ancestor.
 */
function buildFolderTree(folders: FolderType[]): TreeNode[] {
  if (folders.length === 0) return [];

  // Use the first non-null delimiter found; fall back to "/".
  const delimiter = folders.find((f) => f.delimiter)?.delimiter ?? "/";

  // Build a node for every folder.
  const nodeMap = new Map<string, TreeNode>();
  for (const folder of folders) {
    const parts = folder.name.split(delimiter);
    const displayName = parts[parts.length - 1];
    const parentPath =
      parts.length > 1 ? parts.slice(0, -1).join(delimiter) + delimiter : "";
    nodeMap.set(folder.name, {
      folder,
      displayName,
      parentPath,
      depth: parts.length - 1,
      children: [],
    });
  }

  // Link each node to its parent, or add it as a root.
  const roots: TreeNode[] = [];
  for (const folder of folders) {
    const delimIdx = folder.name.lastIndexOf(delimiter);
    if (delimIdx === -1) {
      roots.push(nodeMap.get(folder.name)!);
    } else {
      const parentName = folder.name.slice(0, delimIdx);
      const parent = nodeMap.get(parentName);
      if (parent) {
        parent.children.push(nodeMap.get(folder.name)!);
      } else {
        // Parent not in list — treat as a root (shouldn't normally happen with Dovecot).
        roots.push(nodeMap.get(folder.name)!);
      }
    }
  }

  // Sort all levels: well-known order first, then alphabetically within siblings.
  function sortNodes(nodes: TreeNode[]): TreeNode[] {
    return nodes
      .sort(
        (a, b) =>
          folderSortOrder(a.folder, a.displayName) - folderSortOrder(b.folder, b.displayName) ||
          a.displayName.localeCompare(b.displayName),
      )
      .map((n) => ({ ...n, children: sortNodes(n.children) }));
  }

  return sortNodes(roots);
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

function getFolderIcon(name: string) {
  const lower = name.toLowerCase();
  if (lower === "inbox") return <Inbox className="size-4 shrink-0" />;
  if (lower === "sent" || lower.includes("sent")) return <Send className="size-4 shrink-0" />;
  if (lower === "drafts" || lower.includes("draft")) return <FileText className="size-4 shrink-0" />;
  if (lower === "trash" || lower.includes("trash")) return <Trash2 className="size-4 shrink-0" />;
  if (lower === "junk" || lower === "spam" || lower.includes("junk") || lower.includes("spam"))
    return <AlertCircle className="size-4 shrink-0" />;
  if (lower === "starred" || lower === "flagged") return <Star className="size-4 shrink-0" />;
  return <Folder className="size-4 shrink-0" />;
}

function SkeletonList() {
  return (
    <div className="flex flex-col gap-1 p-2">
      {Array.from({ length: 5 }).map((_, i) => (
        <div key={i} className="h-9 animate-pulse rounded-md bg-sidebar-accent" />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Inline rename input
// ---------------------------------------------------------------------------

function InlineRenameInput({
  currentName,
  displayName,
  parentPath,
  onDone,
}: {
  /** Full IMAP path — used as the `name` argument to the rename mutation. */
  currentName: string;
  /** Last path segment shown in the text field. */
  displayName: string;
  /** Parent path prefix including trailing delimiter (e.g. "Projects/"). Empty for roots. */
  parentPath: string;
  onDone: () => void;
}) {
  const [value, setValue] = useState(displayName);
  const inputRef = useRef<HTMLInputElement>(null);
  const renameFolder = useRenameFolder();

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const handleSubmit = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed || trimmed === displayName) {
      onDone();
      return;
    }
    // Don't re-fire while a request is already in flight (guards against onBlur retrigger).
    if (renameFolder.isPending) return;
    renameFolder.mutate(
      { name: currentName, newName: parentPath + trimmed },
      {
        onSuccess: () => onDone(),
        onError: () => {
          // Keep input open so the user can correct and retry.
        },
      },
    );
  }, [value, displayName, currentName, parentPath, renameFolder, onDone]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleSubmit();
      } else if (e.key === "Escape") {
        onDone();
      }
    },
    [handleSubmit, onDone],
  );

  return (
    <div className="flex w-full items-center gap-1 py-1 pr-2">
      <input
        ref={inputRef}
        type="text"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={handleKeyDown}
        onBlur={handleSubmit}
        className={cn(
          "flex-1 rounded border border-input bg-background px-2 py-1 text-sm text-foreground",
          "outline-none focus:border-ring focus:ring-1 focus:ring-ring/50",
        )}
        disabled={renameFolder.isPending}
        autoComplete="off"
        spellCheck={false}
      />
      {renameFolder.isPending ? (
        <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
      ) : (
        <>
          <button
            type="button"
            onClick={handleSubmit}
            className="rounded p-0.5 text-muted-foreground hover:text-foreground"
            aria-label="Confirm rename"
          >
            <Check className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={onDone}
            className="rounded p-0.5 text-muted-foreground hover:text-foreground"
            aria-label="Cancel rename"
          >
            <X className="size-3.5" />
          </button>
        </>
      )}
      {renameFolder.isError && (
        <span className="text-xs text-destructive" title={renameFolder.error?.message}>
          Error
        </span>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Single folder row
// ---------------------------------------------------------------------------

function FolderItem({
  folder,
  displayName,
  depth,
  hasChildren,
  isCollapsed,
  onToggleCollapse,
  isRenaming,
  onStartRename,
  onEndRename,
}: {
  folder: FolderType;
  /** Last path segment to display in the sidebar. */
  displayName: string;
  /** 0-based nesting depth — controls left indentation. */
  depth: number;
  hasChildren: boolean;
  isCollapsed: boolean;
  onToggleCollapse: () => void;
  isRenaming: boolean;
  onStartRename: () => void;
  onEndRename: () => void;
}) {
  const openMenuRef = useRef<((pos: { x: number; y: number }) => void) | null>(null);
  const activeFolder = useUiStore((s) => s.activeFolder);
  const setActiveFolder = useUiStore((s) => s.setActiveFolder);
  const shouldAnimate = useUiStore((s) => s.effectiveAnimationMode) !== "off";
  const isActive = activeFolder === folder.name;
  const isFetching = useIsFetching({ queryKey: ["messages", folder.name] });
  const moveMessage = useMoveMessage();
  const [isDragOver, setIsDragOver] = useState(false);
  const dragCounter = useRef(0);

  // \Noselect folders are path-only containers — they hold no messages.
  const isNoSelect = folder.attributes.some(
    (a) => a.toLowerCase() === "\\noselect",
  );

  // Left indent: 8px base + 8px per additional depth level.
  // The chevron/spacer (24px wide) sits after this indent before the icon.
  const indentPx = 8 + depth * 8;

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      if (isNoSelect) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
    },
    [isNoSelect],
  );

  const handleDragEnter = useCallback(
    (e: React.DragEvent) => {
      if (isNoSelect) return;
      e.preventDefault();
      dragCounter.current += 1;
      setIsDragOver(true);
    },
    [isNoSelect],
  );

  const handleDragLeave = useCallback(() => {
    dragCounter.current -= 1;
    if (dragCounter.current <= 0) {
      dragCounter.current = 0;
      setIsDragOver(false);
    }
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      if (isNoSelect) return;
      e.preventDefault();
      dragCounter.current = 0;
      setIsDragOver(false);
      try {
        const raw = e.dataTransfer.getData("application/json");
        if (!raw) return;
        const data = JSON.parse(raw) as { uid: number; folder: string; subject: string };
        if (data.folder === folder.name) return;
        moveMessage.mutate({ fromFolder: data.folder, toFolder: folder.name, uid: data.uid });
      } catch {
        // ignore malformed drag payload
      }
    },
    [folder.name, isNoSelect, moveMessage],
  );

  const isDrafts = isDraftsFolder(folder.name);
  const badgeCount = isDrafts ? folder.total_count : folder.unread_count;

  // ----- Rename state -----
  if (isRenaming) {
    return (
      <div
        className="flex items-center rounded-md bg-sidebar-accent"
        style={{ paddingLeft: `${indentPx}px` }}
      >
        {/* Same-width spacer as the chevron so the icon aligns with the normal state. */}
        <span className="size-6 shrink-0" aria-hidden />
        {getFolderIcon(folder.name)}
        <InlineRenameInput
          currentName={folder.name}
          displayName={displayName}
          parentPath={folder.name.slice(0, folder.name.length - displayName.length)}
          onDone={onEndRename}
        />
      </div>
    );
  }

  // ----- Normal state -----
  // Two-button layout keeps the collapse toggle and the folder navigation as
  // separate interactive elements (HTML does not allow nesting buttons).
  const row = (
    <div
      className={cn(
        "flex items-center rounded-md transition-colors",
        isDragOver || isActive
          ? "bg-primary/10 active:bg-primary/15"
          : "hover:bg-sidebar-foreground/10 active:bg-sidebar-foreground/15",
      )}
      style={{ paddingLeft: `${indentPx}px` }}
    >
      {/* Expand/collapse toggle, or an invisible spacer to keep icons aligned. */}
      {hasChildren ? (
        <button
          type="button"
          onClick={onToggleCollapse}
          aria-label={isCollapsed ? "Expand folder" : "Collapse folder"}
          aria-expanded={!isCollapsed}
          className="flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground hover:text-foreground"
        >
          <ChevronRight
            className={cn(
              "size-3",
              shouldAnimate && "transition-transform duration-150",
              !isCollapsed && "rotate-90",
            )}
          />
        </button>
      ) : (
        <span className="size-6 shrink-0" aria-hidden />
      )}

      {/* Folder navigation button. */}
      <button
        type="button"
        onClick={() => {
          if (!isNoSelect) setActiveFolder(folder.name);
        }}
        aria-current={isActive ? "page" : undefined}
        className={cn(
          "flex flex-1 items-center gap-2 py-2 pr-3 text-sm",
          isDragOver || isActive
            ? "font-semibold text-primary"
            : isNoSelect
              ? "cursor-default text-muted-foreground"
              : "font-medium text-sidebar-foreground",
        )}
      >
        {getFolderIcon(folder.name)}
        <span className="flex-1 truncate text-left">
          {/*
           * Use formatFolderName on the full IMAP name first — this handles nested
           * system folders like [Gmail]/All Mail → "Archive". If the full name was
           * not recognised (function returns it unchanged), fall back to the last
           * path segment so custom subfolders show "Work" not "Projects/Work".
           */}
          {(() => { const pretty = formatFolderName(folder.name); return pretty !== folder.name ? pretty : displayName; })()}
        </span>
        {!isNoSelect && badgeCount > 0 ? (
          <span
            className={cn(
              "min-w-[20px] rounded-full px-1.5 py-0.5 text-center text-xs font-semibold",
              isDrafts
                ? "bg-muted-foreground/20 text-muted-foreground"
                : "bg-primary text-primary-foreground",
            )}
          >
            {badgeCount}
          </span>
        ) : isFetching > 0 && !isNoSelect ? (
          <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
        ) : null}
      </button>
      {!isNoSelect && (
        <FolderRowMenu
          folder={folder}
          onOpen={(pos) => openMenuRef.current?.(pos)}
        />
      )}
    </div>
  );

  // \Noselect folders cannot receive drag-drops and have no context menu.
  if (isNoSelect) return row;

  return (
    <FolderContextMenu
      folderName={folder.name}
      onRename={onStartRename}
      onDragOver={handleDragOver}
      onDragEnter={handleDragEnter}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      onOpenMenu={(handler) => { openMenuRef.current = handler; }}
    >
      {row}
    </FolderContextMenu>
  );
}

// ---------------------------------------------------------------------------
// Recursive tree node renderer
// ---------------------------------------------------------------------------

function FolderTreeNodeItem({
  node,
  renamingFolder,
  setRenamingFolder,
  collapsed,
  onToggle,
}: {
  node: TreeNode;
  renamingFolder: string | null;
  setRenamingFolder: (name: string | null) => void;
  collapsed: Set<string>;
  onToggle: (name: string) => void;
}) {
  const isCollapsed = collapsed.has(node.folder.name);
  const hasChildren = node.children.length > 0;

  return (
    <>
      <FolderItem
        folder={node.folder}
        displayName={node.displayName}
        depth={node.depth}
        hasChildren={hasChildren}
        isCollapsed={isCollapsed}
        onToggleCollapse={() => onToggle(node.folder.name)}
        isRenaming={renamingFolder === node.folder.name}
        onStartRename={() => setRenamingFolder(node.folder.name)}
        onEndRename={() => setRenamingFolder(null)}
      />
      {!isCollapsed &&
        node.children.map((child) => (
          <FolderTreeNodeItem
            key={child.folder.name}
            node={child}
            renamingFolder={renamingFolder}
            setRenamingFolder={setRenamingFolder}
            collapsed={collapsed}
            onToggle={onToggle}
          />
        ))}
    </>
  );
}

// ---------------------------------------------------------------------------
// Mailbox space indicator
// ---------------------------------------------------------------------------

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function MailboxSpace({
  usageBytes,
  limitBytes,
}: {
  usageBytes: number;
  limitBytes: number | null;
}) {
  const unlimited = !limitBytes || limitBytes <= 0;

  if (unlimited) {
    return (
      <div className="mt-1 px-1">
        <p className="text-[11px] leading-none text-muted-foreground">
          {formatBytes(usageBytes)} used
        </p>
      </div>
    );
  }

  const pct = Math.min((usageBytes / limitBytes) * 100, 100);
  const isHigh = pct >= 90;
  const isMedium = pct >= 75;

  return (
    <div className="mt-1 px-1">
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
        <div
          className={cn(
            "h-full rounded-full transition-all",
            isHigh ? "bg-destructive" : isMedium ? "bg-yellow-500" : "bg-primary/60",
          )}
          style={{ width: `${pct}%` }}
        />
      </div>
      <p
        className={cn(
          "mt-1 text-[11px] leading-none",
          isHigh ? "text-destructive" : "text-muted-foreground",
        )}
      >
        {formatBytes(usageBytes)} of {formatBytes(limitBytes)} used
      </p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// FolderTree (root component)
// ---------------------------------------------------------------------------

export function FolderTree() {
  const { data, isLoading, isError, refetch } = useFolders();
  const { data: quota } = useQuota();
  const activeFolder = useUiStore((s) => s.activeFolder);
  const setViewMode = useUiStore((s) => s.setViewMode);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [renamingFolder, setRenamingFolder] = useState<string | null>(null);
  // Set of folder names that are currently collapsed.  Empty = all expanded.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const toggleCollapsed = useCallback((name: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(name)) {
        next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  }, []);

  // Build the folder tree only when the API data changes.
  const tree = useMemo(
    () => (data ? buildFolderTree(data.folders) : []),
    [data],
  );

  return (
    <div className="flex h-full flex-col">
      {/* Account switcher */}
      <div className="border-b border-sidebar-border p-2">
        <AccountSwitcher />
      </div>

      {/* Folder list */}
      <nav className="flex-1 overflow-y-auto">
        {isLoading && <SkeletonList />}

        {isError && (
          <div className="flex flex-col items-center gap-3 px-4 py-8 text-center">
            <p className="text-sm text-muted-foreground">Failed to load folders</p>
            <Button variant="outline" size="sm" onClick={() => refetch()}>
              Retry
            </Button>
          </div>
        )}

        {data && (
          <div className="flex flex-col gap-0.5 px-1 py-1">
            {tree.map((node) => (
              <FolderTreeNodeItem
                key={node.folder.name}
                node={node}
                renamingFolder={renamingFolder}
                setRenamingFolder={setRenamingFolder}
                collapsed={collapsed}
                onToggle={toggleCollapsed}
              />
            ))}
          </div>
        )}

        {/* Tags section */}
        <TagSection />
      </nav>

      {/* New folder button + settings + mailbox space */}
      <div className="border-t border-sidebar-border p-2">
        <Button
          variant="ghost"
          size="sm"
          className="w-full justify-start gap-2 text-muted-foreground hover:text-foreground"
          onClick={() => setCreateDialogOpen(true)}
        >
          <FolderPlus className="size-4" />
          New folder
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="md:hidden w-full justify-start gap-2 text-muted-foreground hover:text-foreground"
          onClick={() => setViewMode("settings")}
        >
          <Settings className="size-4" />
          Settings
        </Button>
        {quota?.usage_bytes != null && (
          <>
            <div className="my-1.5 border-t border-sidebar-border" />
            <MailboxSpace usageBytes={quota.usage_bytes} limitBytes={quota.limit_bytes} />
          </>
        )}
      </div>

      {/* Create folder dialog */}
      <CreateFolderDialog
        open={createDialogOpen}
        onClose={() => setCreateDialogOpen(false)}
      />
    </div>
  );
}
