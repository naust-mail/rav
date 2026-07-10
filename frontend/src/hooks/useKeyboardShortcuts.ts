"use client";

import { useEffect, useCallback, useMemo } from "react";
import { useUiStore } from "@/stores/useUiStore";
import {
  useUpdateFlags,
  useMoveMessage,
  useDeleteMessage,
  useMessages,
  useMessage,
} from "@/hooks/useMessages";
import { useSearch } from "@/hooks/useSearch";
import {
  isValidCommittedSearch,
  normalizeSearchQuery,
} from "@/lib/search-parser";
import type { SearchResultItem } from "@/types/message";
import { useComposeStore } from "@/stores/useComposeStore";
import { useIdentities } from "@/hooks/useIdentities";
import {
  extractHeader,
  buildReplySubject,
  buildForwardSubject,
  buildReplyQuoteHtml,
  buildReplyQuoteText,
  buildForwardBody,
  buildForwardBodyHtml,
  buildReferences,
} from "@/lib/email-utils";
function isInputFocused(): boolean {
  const el = document.activeElement;
  if (!el) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === "input" || tag === "textarea" || tag === "select") return true;
  if ((el as HTMLElement).isContentEditable) return true;
  return false;
}

export function useKeyboardShortcuts() {
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedMessageUid = useUiStore((s) => s.selectedMessageUid);
  const selectMessage = useUiStore((s) => s.selectMessage);
  const setActiveFolder = useUiStore((s) => s.setActiveFolder);
  const searchActive = useUiStore((s) => s.searchActive);
  const searchQuery = useUiStore((s) => s.searchQuery);
  const searchSortOrder = useUiStore((s) => s.searchSortOrder);
  const clearSearch = useUiStore((s) => s.clearSearch);
  const setShortcutsOpen = useUiStore((s) => s.setShortcutsOpen);
  const setCommandPaletteOpen = useUiStore((s) => s.setCommandPaletteOpen);

  const updateFlags = useUpdateFlags();
  const moveMessage = useMoveMessage();
  const deleteMessage = useDeleteMessage();
  const { data } = useMessages(activeFolder);
  const { data: messageData } = useMessage(activeFolder, selectedMessageUid ?? 0);
  const { data: identities } = useIdentities();

  const normalizedQuery = normalizeSearchQuery(searchQuery);
  const hasActiveSearch = searchActive && isValidCommittedSearch(normalizedQuery);

  const { data: searchData } = useSearch(
    hasActiveSearch ? searchQuery : "",
    undefined,
    searchSortOrder,
  );
  const searchResults = useMemo(
    () => searchData?.pages.flatMap((p) => p.results) ?? [],
    [searchData?.pages],
  );

  const folderMessages = useMemo(
    () => data?.pages.flatMap((page) => page.messages) ?? [],
    [data?.pages],
  );

  const findCurrentSearchIndex = useCallback((): number => {
    if (selectedMessageUid === null) return -1;
    return searchResults.findIndex(
      (r) => r.folder_name === activeFolder && r.uid === selectedMessageUid,
    );
  }, [searchResults, activeFolder, selectedMessageUid]);

  const selectSearchResult = useCallback(
    (result: SearchResultItem) => {
      setActiveFolder(result.folder_name);
      selectMessage(result.uid);
      (document.activeElement as HTMLElement)?.blur?.();
      useUiStore.getState().setKeyboardNav(true);
    },
    [setActiveFolder, selectMessage],
  );

  const getSearchNavigationTarget = useCallback(
    (direction: "next" | "prev"): SearchResultItem | null => {
      if (searchResults.length === 0) return null;

      const currentIndex = findCurrentSearchIndex();

      if (currentIndex === -1) {
        return direction === "next"
          ? searchResults[0]
          : searchResults[searchResults.length - 1];
      }

      const nextIndex =
        direction === "next" ? currentIndex + 1 : currentIndex - 1;

      if (nextIndex < 0 || nextIndex >= searchResults.length) {
        return null;
      }

      return searchResults[nextIndex];
    },
    [searchResults, findCurrentSearchIndex],
  );

  useEffect(() => {
    const hasModifier = (e: KeyboardEvent) => e.metaKey || e.ctrlKey || e.altKey;

    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === "k") {
        e.preventDefault();
        setTimeout(() => {
          const searchInput = document.querySelector(
            "[data-search-input]",
          ) as HTMLElement;
          searchInput?.focus();
        }, 0);
        return;
      }

      if ((e.metaKey || e.ctrlKey) && e.key === "p") {
        e.preventDefault();
        setCommandPaletteOpen(true);
        return;
      }

      if ((e.metaKey || e.ctrlKey) && e.key === "a" && searchActive) {
        e.preventDefault();
        const searchInput = document.querySelector("[data-search-input]") as HTMLInputElement;
        if (searchInput) {
          searchInput.focus();
          searchInput.select();
        }
        return;
      }

      const searchInput = document.querySelector("[data-search-input]");
      const isArrowKey = e.key === "ArrowDown" || e.key === "ArrowUp";

      if (hasActiveSearch && isArrowKey && document.activeElement === searchInput) {
        (document.activeElement as HTMLElement)?.blur?.();
      }

      if (hasActiveSearch && document.activeElement === searchInput) {
        return;
      }

      if (isInputFocused() || hasModifier(e)) {
        return;
      }

      if (e.key === "?") {
        e.preventDefault();
        setShortcutsOpen(true);
        return;
      }

      if (e.key === "Escape") {
        if (searchActive) {
          clearSearch();
        } else if (selectedMessageUid !== null) {
          selectMessage(null);
        }
        return;
      }

      if (e.key === "c") {
        e.preventDefault();
        useComposeStore.getState().openCompose();
        return;
      }

      if (selectedMessageUid === null) {
        if (e.key === "j" || e.key === "ArrowDown") {
          if (hasActiveSearch && searchResults.length > 0) {
            e.preventDefault();
            selectSearchResult(searchResults[0]);
          } else if (folderMessages.length > 0) {
            e.preventDefault();
            selectMessage(folderMessages[0].uid);
          }
        }
        if (e.key === "k" || e.key === "ArrowUp") {
          if (hasActiveSearch && searchResults.length > 0) {
            e.preventDefault();
            selectSearchResult(searchResults[searchResults.length - 1]);
          } else if (folderMessages.length > 0) {
            e.preventDefault();
            selectMessage(folderMessages[0].uid);
          }
        }
        return;
      }

      switch (e.key) {
        case "r":
          if (!messageData) break;
          e.preventDefault();
          {
            const messageId = extractHeader(messageData.raw_headers, "Message-ID");
            const refs = extractHeader(messageData.raw_headers, "References");
            const hasHtml = !!(messageData.html && messageData.html.trim());
            const matchedIdentity = (() => {
              if (!identities) return null;
              const emails = [...messageData.to_addresses, ...messageData.cc_addresses].map((a) => a.address.toLowerCase());
              return identities.find((i) => emails.includes(i.email.toLowerCase()))?.id ?? null;
            })();
            useComposeStore.getState().openReply({
              to: messageData.from_address,
              cc: "",
              subject: buildReplySubject(messageData.subject),
              body: hasHtml ? "<p><br></p>" : "",
              quotedHtml: hasHtml ? buildReplyQuoteHtml(messageData.html!, messageData.from_address, messageData.date) : null,
              quotedText: buildReplyQuoteText(messageData.text, messageData.from_address, messageData.date),
              inReplyTo: messageId,
              references: buildReferences(refs, messageId),
              fromIdentityId: matchedIdentity,
              isHtml: hasHtml,
            });
          }
          break;

        case "f":
          if (!messageData) break;
          e.preventDefault();
          {
            const toList = messageData.to_addresses
              .map((a) => (a.name ? `${a.name} <${a.address}>` : a.address))
              .join(", ");
            const hasHtml = !!(messageData.html && messageData.html.trim());
            useComposeStore.getState().openForward({
              subject: buildForwardSubject(messageData.subject),
              body: hasHtml
                ? buildForwardBodyHtml(messageData.html!, messageData.from_address, messageData.date, messageData.subject, toList)
                : buildForwardBody(messageData.text, messageData.from_address, messageData.date, messageData.subject, toList),
              isHtml: hasHtml,
            });
          }
          break;

        case "e":
          if (activeFolder !== "Archive") {
            e.preventDefault();
            moveMessage.mutate({
              fromFolder: activeFolder,
              toFolder: "Archive",
              uid: selectedMessageUid,
            });
          }
          break;

        case "Delete":
        case "Backspace":
          e.preventDefault();
          if (activeFolder === "Trash") {
            deleteMessage.mutate(
              { folder: activeFolder, uid: selectedMessageUid },
            );
          } else {
            moveMessage.mutate(
              {
                fromFolder: activeFolder,
                toFolder: "Trash",
                uid: selectedMessageUid,
              },
            );
          }
          break;

        case "s": {
          const currentMsg = folderMessages.find(
            (m) => m.uid === selectedMessageUid,
          );
          if (currentMsg) {
            const flagged = currentMsg.flags.includes("\\Flagged");
            updateFlags.mutate({
              folder: activeFolder,
              uid: selectedMessageUid,
              flags: ["\\Flagged"],
              add: !flagged,
            });
          }
          break;
        }

        case "u": {
          const currentMsg = folderMessages.find(
            (m) => m.uid === selectedMessageUid,
          );
          if (currentMsg) {
            const seen = currentMsg.flags.includes("\\Seen");
            updateFlags.mutate({
              folder: activeFolder,
              uid: selectedMessageUid,
              flags: ["\\Seen"],
              add: !seen,
            });
          }
          break;
        }

        case "j":
        case "ArrowDown":
          e.preventDefault();
          if (hasActiveSearch) {
            const target = getSearchNavigationTarget("next");
            if (target) selectSearchResult(target);
          } else {
            const currentIndex = folderMessages.findIndex(
              (m) => m.uid === selectedMessageUid,
            );
            if (currentIndex >= 0 && currentIndex < folderMessages.length - 1) {
              selectMessage(folderMessages[currentIndex + 1].uid);
              (document.activeElement as HTMLElement)?.blur?.();
              useUiStore.getState().setKeyboardNav(true);
            }
          }
          break;

        case "k":
        case "ArrowUp":
          e.preventDefault();
          if (hasActiveSearch) {
            const target = getSearchNavigationTarget("prev");
            if (target) selectSearchResult(target);
          } else {
            const currentIndex = folderMessages.findIndex(
              (m) => m.uid === selectedMessageUid,
            );
            if (currentIndex > 0) {
              selectMessage(folderMessages[currentIndex - 1].uid);
              (document.activeElement as HTMLElement)?.blur?.();
              useUiStore.getState().setKeyboardNav(true);
            }
          }
          break;
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [
    activeFolder,
    selectedMessageUid,
    selectMessage,
    searchActive,
    clearSearch,
    setShortcutsOpen,
    setCommandPaletteOpen,
    updateFlags,
    moveMessage,
    deleteMessage,
    folderMessages,
    hasActiveSearch,
    searchResults,
    selectSearchResult,
    getSearchNavigationTarget,
    messageData,
    identities,
  ]);
}
