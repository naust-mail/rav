"use client";

import { useRef, useCallback, useEffect, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AnimatePresence } from "framer-motion";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { PenLine, PanelRight, Inbox, Tag, RefreshCw } from "lucide-react";
import { useMessages, useMessage, useMessageByMessageId } from "@/hooks/useMessages";
import { useTags, useTagMessages } from "@/hooks/useTags";
import { useGetDraftAttachments } from "@/hooks/useCompose";
import { extractHeader, buildReplyQuoteHtml, buildReplyQuoteText } from "@/lib/email-utils";
import { createFadeSlideVariants, type MotionVariants } from "@/lib/motion/variants";
import type { AnimationMode } from "@/lib/motion/config";
import { ANIMATION_MODES } from "@/lib/motion/config";
import { useUiStore } from "@/stores/useUiStore";
import { useComposeStore } from "@/stores/useComposeStore";
import { MessageListItem } from "./MessageListItem";
import { formatFolderName } from "./FolderTree";
import { BulkActionBar } from "./BulkActionBar";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Pre-computed fade-slide variants keyed by animation mode — avoids per-render allocation. */
const ROW_MOTION_VARIANTS_BY_MODE: Record<AnimationMode, MotionVariants> = Object.fromEntries(
  ANIMATION_MODES.map((mode) => [mode, createFadeSlideVariants(mode, "y")]),
) as Record<AnimationMode, MotionVariants>;

function SkeletonRows({ count, height, compact }: { count: number; height: number; compact: boolean }) {
  return (
    <div className="flex flex-col">
      {Array.from({ length: count }).map((_, i) => (
        <div
          key={i}
          className="flex items-center gap-3 border-b border-border px-3"
          style={{ height }}
        >
          <div className="h-2 w-2 animate-pulse rounded-full bg-muted shrink-0" />
          {compact ? (
            <>
              <div className="h-2.5 w-20 animate-pulse rounded bg-muted shrink-0" />
              <div className="h-2.5 flex-1 animate-pulse rounded bg-muted" />
              <div className="h-2.5 w-10 animate-pulse rounded bg-muted shrink-0" />
            </>
          ) : (
            <div className="flex flex-1 flex-col gap-1.5">
              <div className="flex items-center gap-2">
                <div className="h-2.5 w-24 animate-pulse rounded bg-muted" />
                <div className="h-2.5 flex-1 animate-pulse rounded bg-muted" />
                <div className="h-2.5 w-10 animate-pulse rounded bg-muted" />
              </div>
              <div className="h-2 w-3/4 animate-pulse rounded bg-muted/60" />
            </div>
          )}
        </div>
      ))}
    </div>
  );
}


/** Parse the draft UUID from a Message-ID header value like `<uuid@draft>`. */
function parseDraftUuid(rawHeaders: string): string | null {
  const match = rawHeaders.match(/Message-ID:\s*<([^@\s>]+)@draft>/i);
  return match ? match[1] : null;
}

function ToggleReadingPaneButton() {
  const visible = useUiStore((s) => s.readingPaneVisible);
  const setVisible = useUiStore((s) => s.setReadingPaneVisible);

  return (
    <button
      type="button"
      aria-label={visible ? "Hide reading pane" : "Show reading pane"}
      onClick={() => setVisible(!visible)}
      className={cn(
        "hidden md:flex size-6 items-center justify-center rounded transition-colors hover:bg-accent active:bg-accent/70",
        !visible && "text-primary",
      )}
    >
      <PanelRight className="size-3.5" />
    </button>
  );
}

export function MessageList() {
  const activeFolder = useUiStore((s) => s.activeFolder);
  const activeTagId = useUiStore((s) => s.activeTagId);
  const density = useUiStore((s) => s.density);
  const selectedMessageUid = useUiStore((s) => s.selectedMessageUid);
  const selectMessage = useUiStore((s) => s.selectMessage);
  const selectedMessageUids = useUiStore((s) => s.selectedMessageUids);
  const bulkSelectMode = useUiStore((s) => s.bulkSelectMode);
  const keyboardNav = useUiStore((s) => s.keyboardNav);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const toggleBulkSelect = useUiStore((s) => s.toggleBulkSelect);
  const selectAllMessages = useUiStore((s) => s.selectAllMessages);
  const clearBulkSelection = useUiStore((s) => s.clearBulkSelection);
  const shouldAnimateRows = effectiveAnimationMode !== "off";

  const folderQuery = useMessages(activeFolder);
  const tagQuery = useTagMessages(activeTagId);
  const { data: tagsData } = useTags();

  // Select the right data source based on whether a tag filter is active.
  const isTagView = !!activeTagId;
  const data = isTagView ? undefined : folderQuery.data;
  const isLoading = isTagView ? tagQuery.isLoading : folderQuery.isLoading;
  const isFetching = isTagView ? tagQuery.isFetching : folderQuery.isFetching;
  const isError = isTagView ? tagQuery.isError : folderQuery.isError;
  const refetch = isTagView ? tagQuery.refetch : folderQuery.refetch;
  const fetchNextPage = folderQuery.fetchNextPage;
  const hasNextPage = isTagView ? false : folderQuery.hasNextPage;
  const isFetchingNextPage = isTagView ? false : folderQuery.isFetchingNextPage;

  // Flatten all pages into a single array of messages.
  const folderMessages = data?.pages.flatMap((page) => page.messages) ?? [];
  const tagMessages = isTagView ? (tagQuery.data?.messages ?? []) : [];
  const messages = isTagView ? tagMessages : folderMessages;
  const totalCount = isTagView
    ? (tagQuery.data?.total_count ?? 0)
    : (data?.pages[0]?.total_count ?? 0);
  const isSyncing = data?.pages[0]?.syncing ?? false;

  // Resolve tag name for header display.
  const activeTagName = tagsData?.tags.find((t) => t.id === activeTagId)?.name;

  // ----- Draft open state machine -----
  // Step 1: click draft → fetch message detail → extract UUID + In-Reply-To → trigger step 2.
  // Step 2: wait for attachments + original message (if reply) → build quote → open compose.
  const openDraft = useComposeStore((s) => s.openDraft);
  const [pendingDraft, setPendingDraft] = useState<{ folder: string; uid: number } | null>(null);
  const [pendingDraftUuid, setPendingDraftUuid] = useState<string | null>(null);
  // Message-ID of the original message being replied to, set in step 1, cleared in step 2.
  const [pendingInReplyToId, setPendingInReplyToId] = useState<string | null>(null);
  // Holds the fetched detail between step 1 and step 2.
  const pendingDetailRef = useRef<import("@/types/message").MessageDetail | null>(null);

  const messageDetail = useMessage(pendingDraft?.folder ?? "", pendingDraft?.uid ?? 0);
  const draftAttachments = useGetDraftAttachments(pendingDraftUuid);
  // Fetch the original message being replied to so we can reconstruct the quote on reopen.
  const originalMessage = useMessageByMessageId(pendingInReplyToId);

  // Step 1: message detail arrives - save to ref, extract UUID + reply context.
  useEffect(() => {
    if (!pendingDraft || !messageDetail.data) return;
    const detail = messageDetail.data;
    pendingDetailRef.current = detail;
    setPendingDraft(null); // eslint-disable-line react-hooks/set-state-in-effect
    const uuid = parseDraftUuid(detail.raw_headers);
    if (uuid) {
      const inReplyTo = extractHeader(detail.raw_headers, "In-Reply-To");
      setPendingInReplyToId(inReplyTo || null); // eslint-disable-line react-hooks/set-state-in-effect
      setPendingDraftUuid(uuid); // eslint-disable-line react-hooks/set-state-in-effect -- triggers step 2
    } else {
      // No UUID (not saved via our API) - open with what we have, no quote.
      pendingDetailRef.current = null;
      openDraft({
        id: detail.uid.toString(),
        to: detail.to_addresses.map((a) => a.address).join(", "),
        cc: detail.cc_addresses.map((a) => a.address).join(", "),
        bcc: extractHeader(detail.raw_headers, "Bcc") ?? "",
        subject: detail.subject,
        body: detail.html ?? detail.text ?? "",
        inReplyTo: null,
        references: null,
        attachments: [],
        isHtml: !!detail.html,
      });
    }
  }, [pendingDraft, messageDetail.data, openDraft]);

  // Step 2: wait for attachments + original message (if reply), then open compose.
  useEffect(() => {
    if (!pendingDraftUuid || draftAttachments.isPending) return;
    // If waiting on the original message for quote reconstruction, hold off.
    if (pendingInReplyToId && originalMessage.isPending) return;

    const uuid = pendingDraftUuid;
    const detail = pendingDetailRef.current;
    if (!detail) return;
    pendingDetailRef.current = null;
    setPendingDraftUuid(null); // eslint-disable-line react-hooks/set-state-in-effect
    setPendingInReplyToId(null); // eslint-disable-line react-hooks/set-state-in-effect

    const orig = originalMessage.data ?? null;
    const hasOrigHtml = !!(orig?.html && orig.html.trim());

    openDraft({
      id: uuid,
      to: detail.to_addresses.map((a) => a.address).join(", "),
      cc: detail.cc_addresses.map((a) => a.address).join(", "),
      bcc: extractHeader(detail.raw_headers, "Bcc") ?? "",
      subject: detail.subject,
      body: detail.html ?? detail.text ?? "",
      inReplyTo: extractHeader(detail.raw_headers, "In-Reply-To") || null,
      references: extractHeader(detail.raw_headers, "References") || null,
      attachments: (draftAttachments.data?.attachments ?? []).map((a) => ({
        id: a.id,
        filename: a.filename,
        contentType: a.content_type,
        size: a.size,
      })),
      isHtml: !!detail.html,
      quotedHtml: orig && hasOrigHtml
        ? buildReplyQuoteHtml(orig.html!, orig.from_address, orig.date)
        : null,
      quotedText: orig ? buildReplyQuoteText(orig.text, orig.from_address, orig.date) : null,
    });
  }, [
    pendingDraftUuid,
    draftAttachments.isPending,
    draftAttachments.data,
    pendingInReplyToId,
    originalMessage.isPending,
    originalMessage.data,
    openDraft,
  ]);

  const parentRef = useRef<HTMLDivElement>(null);
  const rowHeight = density === "compact" ? 36 : 64;
  const prevSelectedUidRef = useRef<number | null>(null);

  // eslint-disable-next-line react-hooks/incompatible-library -- useVirtualizer is designed for this usage
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 10,
  });

  // Scroll the virtualizer to keep the keyboard-selected message visible
  // with a 3-row buffer so the user can preview upcoming messages.
  // Only runs when selectedMessageUid CHANGES (keyboard nav), not on every render.
  useEffect(() => {
    if (selectedMessageUid == null || selectedMessageUid === prevSelectedUidRef.current) return;
    prevSelectedUidRef.current = selectedMessageUid;

    const idx = messages.findIndex((m) => m.uid === selectedMessageUid);
    if (idx < 0) return;

    const scrollEl = parentRef.current;
    if (!scrollEl) return;

    const itemTop = idx * rowHeight;
    const itemBottom = itemTop + rowHeight;
    const viewTop = scrollEl.scrollTop;
    const viewBottom = viewTop + scrollEl.clientHeight;
    const buffer = rowHeight * 3;

    if (itemTop < viewTop + buffer) {
      scrollEl.scrollTop = Math.max(0, itemTop - buffer);
    } else if (itemBottom > viewBottom - buffer) {
      scrollEl.scrollTop = itemBottom - scrollEl.clientHeight + buffer;
    }
  }, [selectedMessageUid, messages, rowHeight]);

  // Fetch next page when scrolling near the bottom.
  const virtualItems = virtualizer.getVirtualItems();
  const rowMotionVariants = ROW_MOTION_VARIANTS_BY_MODE[effectiveAnimationMode];
  const prevVisibleUidsRef = useRef<Set<number>>(new Set());

  const changedVisibleDelays = new Map<number, number>();
  if (shouldAnimateRows) {
    const previous = prevVisibleUidsRef.current;
    let changedVisibleIndex = 0;
    for (const item of virtualItems) {
      const message = messages[item.index];
      if (!message) continue;
      if (!previous.has(message.uid)) {
        const boundedIndex = Math.min(changedVisibleIndex, 6);
        changedVisibleDelays.set(message.uid, boundedIndex * 0.03);
        changedVisibleIndex += 1;
      }
    }
  }

  useEffect(() => {
    const nextVisible = new Set<number>();
    for (const item of virtualItems) {
      const message = messages[item.index];
      if (message) {
        nextVisible.add(message.uid);
      }
    }
    prevVisibleUidsRef.current = nextVisible;
  }, [messages, virtualItems]);

  const lastItem = virtualItems[virtualItems.length - 1];
  const lastItemIndex = lastItem?.index;

  useEffect(() => {
    if (lastItemIndex == null) return;
    if (lastItemIndex >= messages.length - 10 && hasNextPage && !isFetchingNextPage) {
      fetchNextPage();
    }
  }, [lastItemIndex, messages.length, hasNextPage, isFetchingNextPage, fetchNextPage]);

  // Anchor UID for shift-click range selection.
  const anchorUidRef = useRef<number | null>(null);

  const handleClick = useCallback(
    (uid: number, e: React.MouseEvent) => {
      const isMod = e.metaKey || e.ctrlKey;
      const isShift = e.shiftKey;

      if (isMod) {
        // Cmd/Ctrl+click: toggle this message in bulk selection.
        // If entering bulk mode while reading a message, include the read message too.
        if (selectedMessageUids.length === 0 && selectedMessageUid != null && selectedMessageUid !== uid) {
          selectAllMessages([selectedMessageUid, uid]);
        } else {
          toggleBulkSelect(uid);
        }
        anchorUidRef.current = uid;
        return;
      }

      if (isShift && anchorUidRef.current != null) {
        // Shift+click: select range from anchor to clicked message.
        const uids = messages.map((m) => m.uid);
        const anchorIdx = uids.indexOf(anchorUidRef.current);
        const clickedIdx = uids.indexOf(uid);
        if (anchorIdx !== -1 && clickedIdx !== -1) {
          const start = Math.min(anchorIdx, clickedIdx);
          const end = Math.max(anchorIdx, clickedIdx);
          selectAllMessages(uids.slice(start, end + 1));
        }
        return;
      }

      // Plain click: normal single-message selection, clear bulk.
      clearBulkSelection();
      selectMessage(uid);
      anchorUidRef.current = uid;
    },
    [selectMessage, toggleBulkSelect, selectAllMessages, clearBulkSelection, messages, selectedMessageUid, selectedMessageUids],
  );

  const allUids = messages.map((m) => m.uid);
  const allSelected =
    messages.length > 0 && selectedMessageUids.length === messages.length;

  const handleSelectAllToggle = useCallback(() => {
    if (allSelected) {
      clearBulkSelection();
    } else {
      selectAllMessages(allUids);
    }
  }, [allSelected, allUids, clearBulkSelection, selectAllMessages]);

  return (
    <div className="flex h-full flex-col">
      {/* Header bar */}
      <div className="flex shrink-0 items-center justify-between border-b border-border px-4 py-2">
        <div className="flex items-center gap-2">
          {/* Select all checkbox */}
          {!isLoading && !isError && (
            <button
              type="button"
              aria-label={allSelected ? "Deselect all" : "Select all"}
              disabled={messages.length === 0}
              onClick={handleSelectAllToggle}
              className={cn(
                "flex size-4 shrink-0 items-center justify-center rounded border transition-colors transition-opacity",
                messages.length === 0 ? 'opacity-50'
                  : allSelected
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-muted-foreground/40 bg-transparent hover:border-primary",
              )}
            >
              {allSelected && (
                <svg
                  className="size-3"
                  viewBox="0 0 12 12"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M2.5 6l2.5 2.5 4.5-5" />
                </svg>
              )}
            </button>
          )}
          <h2 className="text-sm font-semibold">
            {isTagView ? `Tag: ${activeTagName ?? "..."}` : formatFolderName(activeFolder)}
          </h2>
        </div>
        <div className="flex items-center gap-1.5">
          <span className="text-xs text-muted-foreground">
            {isLoading && !data ? "\u2026" : `${totalCount} messages`}
          </span>
          <button
            type="button"
            aria-label="Refresh"
            onClick={() => refetch()}
            disabled={isFetching}
            className="hidden md:flex size-6 items-center justify-center rounded transition-colors hover:bg-accent disabled:opacity-50"
          >
            <RefreshCw className={cn("size-3.5", (isFetching || isSyncing) && "animate-spin", isSyncing && "text-primary")} />
          </button>
          <ToggleReadingPaneButton />
        </div>
      </div>

      {/* Bulk action bar */}
      <BulkActionBar />

      {/* Non-blocking refetch indicator */}
      <div className="relative h-0">
        {isFetching && !isLoading && messages.length > 0 && (
          <div className="absolute inset-x-0 top-0 h-0.5 animate-pulse bg-primary/30" />
        )}
      </div>

      {/* Loading state (true initial load only) */}
      {isLoading && !data && (
        <SkeletonRows count={8} height={rowHeight} compact={density === "compact"} />
      )}

      {/* Error state */}
      {isError && (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-8 text-center">
          <p className="text-sm text-muted-foreground">
            Failed to load messages
          </p>
          <Button variant="outline" size="sm" onClick={() => refetch()}>
            Retry
          </Button>
        </div>
      )}

      {/* Empty state */}
      {!isLoading && !isError && messages.length === 0 && (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-center">
          {isTagView ? (
            <Tag className="size-10 text-muted-foreground/40" strokeWidth={1.25} />
          ) : (
            <Inbox className="size-10 text-muted-foreground/40" strokeWidth={1.25} />
          )}
          <p className="text-sm font-medium text-muted-foreground">
            {isTagView ? "No messages with this tag" : "No messages in this folder"}
          </p>
          {(isTagView || activeFolder === "INBOX") && (
            <button
              type="button"
              onClick={() => useComposeStore.getState().openCompose()}
              className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
            >
              <PenLine className="size-4" />
              Compose
            </button>
          )}
        </div>
      )}

      {/* Scrollable virtualized message list */}
      {!isLoading && !isError && messages.length > 0 && (
        <div
          ref={parentRef}
          className="flex-1 overflow-y-auto"
          onMouseMove={keyboardNav ? () => useUiStore.getState().setKeyboardNav(false) : undefined}
        >
          <div
            style={{
              height: virtualizer.getTotalSize(),
              width: "100%",
              position: "relative",
            }}
          >
            <AnimatePresence initial={false}>
              {virtualItems.map((virtualRow) => {
                const message = messages[virtualRow.index];
                if (!message) return null;

                const isDraftMessage = message.flags.includes("\\Draft");
                const staggerDelay = changedVisibleDelays.get(message.uid) ?? 0;
                const animateValue = {
                  ...rowMotionVariants.animate,
                  transition: {
                    ...(rowMotionVariants.animate as { transition?: object }).transition,
                    delay: staggerDelay,
                  },
                };

                return (
                  <AnimatedDiv
                    key={message.uid}
                    data-testid="message-list-row-transition"
                    data-row-uid={String(message.uid)}
                    data-row-changed={changedVisibleDelays.has(message.uid) ? "true" : "false"}
                    data-row-stagger-delay={String(staggerDelay)}
                    variants={rowMotionVariants}
                    initial={rowMotionVariants.initial}
                    animate={animateValue}
                    exit={rowMotionVariants.exit}
                    exposeMotionProps={false}
                    data-motion-props={JSON.stringify({
                      initial: rowMotionVariants.initial,
                      animate: animateValue,
                      exit: rowMotionVariants.exit,
                    })}
                    style={{
                      position: "absolute",
                      top: virtualRow.start,
                      left: 0,
                      width: "100%",
                      height: virtualRow.size,
                    }}
                  >
                    <MessageListItem
                      message={message}
                      isSelected={selectedMessageUid === message.uid}
                      density={density}
                      onClick={(e) => {
                        if (isDraftMessage) {
                          setPendingDraft({ folder: message.folder, uid: message.uid });
                        } else {
                          handleClick(message.uid, e);
                        }
                      }}
                      bulkSelectMode={bulkSelectMode}
                      isBulkSelected={selectedMessageUids.includes(message.uid)}
                      onBulkToggle={toggleBulkSelect}
                      suppressHover={keyboardNav}
                      effectiveAnimationMode={effectiveAnimationMode}
                    />
                  </AnimatedDiv>
                );
              })}
            </AnimatePresence>
          </div>
          {isFetchingNextPage && (
            <div className="flex items-center justify-center py-2">
              <span className="text-xs text-muted-foreground">Loading more...</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
