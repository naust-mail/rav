"use client";

import { useState, useRef, useEffect } from "react";
import { ChevronLeft, Menu, Reply, MoreHorizontal, Trash2, Archive, Forward, Type, FileCode } from "lucide-react";
import { useUiStore } from "@/stores/useUiStore";
import { useMobileNav } from "@/hooks/useMobileNav";
import { useMessage, useDeleteMessage, useMoveMessage } from "@/hooks/useMessages";
import { useComposeStore } from "@/stores/useComposeStore";
import { formatFolderName } from "@/components/mail/FolderTree";
import {
  extractHeader,
  buildReplySubject,
  buildReplyQuoteHtml,
  buildReplyQuoteText,
  buildReferences,
  buildForwardSubject,
  buildForwardBody,
  buildForwardBodyHtml,
} from "@/lib/email-utils";
import { cn } from "@/lib/utils";

/** Props for MobileHeader - controls which panel variant is rendered. */
type MobileHeaderProps = {
  panel: "list" | "reading";
};

function ListHeader() {
  const { goBack } = useMobileNav();
  const activeFolder = useUiStore((s) => s.activeFolder);

  return (
    <div className="md:hidden flex shrink-0 items-center gap-2 border-b border-border bg-background px-2 py-2">
      <button
        type="button"
        aria-label="Open folders"
        onClick={goBack}
        className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
      >
        <Menu className="size-5" />
      </button>
      <span className="flex-1 truncate text-sm font-semibold">
        {formatFolderName(activeFolder)}
      </span>
    </div>
  );
}

function ReadingHeader() {
  const { goBack } = useMobileNav();
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedMessageUid = useUiStore((s) => s.selectedMessageUid);
  const selectMessage = useUiStore((s) => s.selectMessage);
  const openReply = useComposeStore((s) => s.openReply);
  const openForward = useComposeStore((s) => s.openForward);
  const bodyMode = useUiStore((s) => s.readingBodyMode);
  const showHeaders = useUiStore((s) => s.readingShowHeaders);
  const toggleBodyMode = useUiStore((s) => s.toggleReadingBodyMode);
  const toggleShowHeaders = useUiStore((s) => s.toggleReadingShowHeaders);

  const { data } = useMessage(activeFolder, selectedMessageUid ?? 0);
  const deleteMessage = useDeleteMessage();
  const moveMessage = useMoveMessage();

  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function handleClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [menuOpen]);

  const handleReply = () => {
    if (!data) return;
    const messageId = extractHeader(data.raw_headers, "Message-ID");
    const refs = extractHeader(data.raw_headers, "References");
    const hasHtml = !!(data.html?.trim());
    openReply({
      to: data.from_address,
      cc: "",
      subject: buildReplySubject(data.subject),
      body: hasHtml ? "<p><br></p>" : "",
      quotedHtml: hasHtml ? buildReplyQuoteHtml(data.html!, data.from_address, data.date) : null,
      quotedText: buildReplyQuoteText(data.text, data.from_address, data.date),
      inReplyTo: messageId,
      references: buildReferences(refs, messageId),
      isHtml: hasHtml,
    });
  };

  const handleDelete = () => {
    if (!data) return;
    deleteMessage.mutate({ folder: activeFolder, uid: data.uid });
    selectMessage(null);
    setMenuOpen(false);
  };

  const handleArchive = () => {
    if (!data) return;
    moveMessage.mutate({ fromFolder: activeFolder, toFolder: "Archive", uid: data.uid });
    selectMessage(null);
    setMenuOpen(false);
  };

  const handleForward = () => {
    if (!data) return;
    const toList = data.to_addresses.map((a) => a.address).join(", ");
    const hasHtml = !!(data.html?.trim());
    openForward?.({
      subject: buildForwardSubject(data.subject),
      body: hasHtml
        ? buildForwardBodyHtml(data.html!, data.from_address, data.date, data.subject, toList)
        : buildForwardBody(data.text, data.from_address, data.date, data.subject, toList),
      isHtml: hasHtml,
    });
    setMenuOpen(false);
  };

  const isTrash = activeFolder === "Trash";

  return (
    <div className="md:hidden flex shrink-0 items-center gap-1 border-b border-border bg-background px-2 py-2">
      <button
        type="button"
        aria-label="Back"
        onClick={goBack}
        className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
      >
        <ChevronLeft className="size-5" />
      </button>
      <span className="flex-1 truncate text-sm font-semibold">
        {data?.subject ?? ""}
      </span>
      <button
        type="button"
        aria-label="Reply"
        onClick={handleReply}
        className={cn(
          "flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground",
          !data && "opacity-40 pointer-events-none",
        )}
      >
        <Reply className="size-4" />
      </button>

      {/* More actions menu */}
      <div className="relative" ref={menuRef}>
        <button
          type="button"
          aria-label="More actions"
          onClick={() => setMenuOpen((o) => !o)}
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
        >
          <MoreHorizontal className="size-4" />
        </button>

        {menuOpen && (
          <div className="absolute right-0 top-9 z-50 min-w-[160px] rounded-md border border-border bg-popover py-1 shadow-md">
            <button
              type="button"
              onClick={handleArchive}
              disabled={!data || isTrash}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent active:bg-accent/70 disabled:pointer-events-none disabled:opacity-40"
            >
              <Archive className="size-3.5" />
              Archive
            </button>
            <button
              type="button"
              onClick={handleForward}
              disabled={!data}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent active:bg-accent/70 disabled:pointer-events-none disabled:opacity-40"
            >
              <Forward className="size-3.5" />
              Forward
            </button>
            <div className="my-1 border-t border-border" />
            <button
              type="button"
              onClick={() => { toggleBodyMode(); setMenuOpen(false); }}
              className={cn(
                "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent active:bg-accent/70",
                bodyMode === "plain" && "text-primary",
              )}
            >
              <Type className="size-3.5" />
              {bodyMode === "html" ? "Plain text" : "HTML"}
            </button>
            <button
              type="button"
              onClick={() => { toggleShowHeaders(); setMenuOpen(false); }}
              className={cn(
                "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm transition-colors hover:bg-accent active:bg-accent/70",
                showHeaders && "text-primary",
              )}
            >
              <FileCode className="size-3.5" />
              Headers
            </button>
            <div className="my-1 border-t border-border" />
            <button
              type="button"
              onClick={handleDelete}
              disabled={!data}
              className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm text-destructive transition-colors hover:bg-accent active:bg-accent/70 disabled:pointer-events-none disabled:opacity-40"
            >
              <Trash2 className="size-3.5" />
              {isTrash ? "Delete permanently" : "Move to Trash"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

export function MobileHeader({ panel }: MobileHeaderProps) {
  if (panel === "list") return <ListHeader />;
  return <ReadingHeader />;
}
