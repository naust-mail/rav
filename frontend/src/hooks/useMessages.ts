"use client";

import {
  useQuery,
  useInfiniteQuery,
  useMutation,
  useQueryClient,
  keepPreviousData,
} from "@tanstack/react-query";
import type { InfiniteData } from "@tanstack/react-query";
import { apiGet, apiPatch, apiPost, apiDelete } from "@/lib/api";
import { useWsStatus } from "@/lib/ws-context";
import { useUiStore } from "@/stores/useUiStore";
import { resolveFolderId } from "@/lib/folders";
import type { MessagesResponse, MessageDetail, MessageHeader } from "@/types/message";
import type { BulkMessageOpResponse } from "@/types/generated/BulkMessageOpResponse";

const PER_PAGE = 50;

export function useMessages(folder: string) {
  const queryClient = useQueryClient();
  const { status } = useWsStatus();
  return useInfiniteQuery({
    queryKey: ["messages", folder],
    queryFn: ({ pageParam = 0 }) =>
      apiGet<MessagesResponse>(
        `/folders/${encodeURIComponent(resolveFolderId(queryClient, folder))}/messages?page=${pageParam}&per_page=${PER_PAGE}`,
      ),
    initialPageParam: 0,
    getNextPageParam: (lastPage) => {
      const fetched = (lastPage.page + 1) * lastPage.per_page;
      return fetched < lastPage.total_count ? lastPage.page + 1 : undefined;
    },
    enabled: !!folder,
    refetchInterval: status === "connected" ? false : 60_000,
    placeholderData: keepPreviousData,
  });
}

export function useMessage(folder: string, uid: number) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: ["message", folder, uid],
    queryFn: () =>
      apiGet<MessageDetail>(
        `/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/${uid}`,
      ),
    enabled: !!folder && uid > 0,
    retry: (failureCount, error) => {
      // Don't retry "not found" — the message was deleted from IMAP.
      if (error instanceof Error && error.message.includes("not found")) return false;
      return failureCount < 2;
    },
  });
}

/** Look up a message by its Message-ID header. Returns null if not in the local cache. */
export function useMessageByMessageId(messageId: string | null) {
  return useQuery({
    queryKey: ["message-by-id", messageId],
    queryFn: () =>
      apiPost<MessageDetail>(`/messages/by-message-id`, { message_id: messageId! }).catch(
        () => null,
      ),
    enabled: !!messageId,
    retry: false,
  });
}

export function useUpdateFlags() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      folder,
      uid,
      flags,
      add,
    }: {
      folder: string;
      uid: number;
      flags: string[];
      add: boolean;
    }) =>
      apiPatch(`/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/${uid}/flags`, {
        flags,
        add,
      }),
    onSuccess: (_, { folder, uid }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", folder] });
      queryClient.invalidateQueries({ queryKey: ["message", folder, uid] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

export function useMoveMessage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      fromFolder,
      toFolder,
      uid,
    }: {
      fromFolder: string;
      toFolder: string;
      uid: number;
    }) =>
      apiPost("/messages/move", {
        from_folder: resolveFolderId(queryClient, fromFolder),
        to_folder: resolveFolderId(queryClient, toFolder),
        uid,
      }),
    onMutate: async ({ fromFolder, toFolder, uid }) => {
      // Auto-advance: if the moved message is selected, select the next (or previous) message.
      const { selectedMessageUid, selectMessage } = useUiStore.getState();
      if (selectedMessageUid === uid) {
        const prev = queryClient.getQueryData<InfiniteData<MessagesResponse>>(["messages", fromFolder]);
        if (prev) {
          const allMessages = prev.pages.flatMap((p) => p.messages);
          const idx = allMessages.findIndex((m) => m.uid === uid);
          const nextMsg = allMessages[idx + 1] ?? allMessages[idx - 1] ?? null;
          selectMessage(nextMsg?.uid ?? null);
        } else {
          selectMessage(null);
        }
      }

      // Cancel in-flight fetches so they don't overwrite our optimistic update.
      await Promise.all([
        queryClient.cancelQueries({ queryKey: ["messages", fromFolder] }),
        queryClient.cancelQueries({ queryKey: ["messages", toFolder] })
      ]);

      const prevFrom = queryClient.getQueryData<InfiniteData<MessagesResponse>>(
        ["messages", fromFolder],
      );
      const prevTo = queryClient.getQueryData<InfiniteData<MessagesResponse>>(
        ["messages", toFolder],
      );

      // Find the message in the source folder cache.
      let movedMsg: MessageHeader | undefined;
      if (prevFrom) {
        for (const page of prevFrom.pages) {
          movedMsg = page.messages.find((m) => m.uid === uid);
          if (movedMsg) break;
        }
      }

      // Remove from source folder cache.
      if (prevFrom) {
        queryClient.setQueryData<InfiniteData<MessagesResponse>>(
          ["messages", fromFolder],
          {
            ...prevFrom,
            pages: prevFrom.pages.map((page) => ({
              ...page,
              messages: page.messages.filter((m) => m.uid !== uid),
              total_count: Math.max(0, page.total_count - 1),
            })),
          },
        );
      }

      // Insert into destination folder cache (first page) with a placeholder
      // UID. The background refetch will reconcile with the real UID.
      if (movedMsg && prevTo) {
        const entry: MessageHeader = {
          ...movedMsg,
          folder_id: resolveFolderId(queryClient, toFolder),
          folder_name: toFolder,
        };
        queryClient.setQueryData<InfiniteData<MessagesResponse>>(
          ["messages", toFolder],
          {
            ...prevTo,
            pages: prevTo.pages.map((page, i) =>
              i === 0
                ? {
                  ...page,
                  messages: [entry, ...page.messages],
                  total_count: page.total_count + 1,
                }
                : { ...page, total_count: page.total_count + 1 },
            ),
          },
        );
      }

      return { prevFrom, prevTo };
    },
    onError: (_err, { fromFolder, toFolder }, context) => {
      // Rollback on failure.
      if (context?.prevFrom) {
        queryClient.setQueryData(["messages", fromFolder], context.prevFrom);
      }
      if (context?.prevTo) {
        queryClient.setQueryData(["messages", toFolder], context.prevTo);
      }
    },
    onSettled: (_data, _err, { fromFolder, toFolder }) => {
      // Always refetch to reconcile with server state.
      queryClient.invalidateQueries({ queryKey: ["messages", fromFolder] });
      queryClient.invalidateQueries({ queryKey: ["messages", toFolder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

export function useDeleteMessage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ folder, uid }: { folder: string; uid: number }) =>
      apiDelete(`/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/${uid}`),
    onMutate: async ({ folder, uid }) => {
      // Auto-advance: if the deleted message is selected, select the next (or previous) message.
      const { selectedMessageUid, selectMessage } = useUiStore.getState();
      if (selectedMessageUid === uid) {
        const prev = queryClient.getQueryData<InfiniteData<MessagesResponse>>(["messages", folder]);
        if (prev) {
          const allMessages = prev.pages.flatMap((p) => p.messages);
          const idx = allMessages.findIndex((m) => m.uid === uid);
          const nextMsg = allMessages[idx + 1] ?? allMessages[idx - 1] ?? null;
          selectMessage(nextMsg?.uid ?? null);
        } else {
          selectMessage(null);
        }
      }

      // Optimistic removal from cache.
      await queryClient.cancelQueries({ queryKey: ["messages", folder] });
      const prev = queryClient.getQueryData<InfiniteData<MessagesResponse>>(
        ["messages", folder],
      );
      if (prev) {
        queryClient.setQueryData<InfiniteData<MessagesResponse>>(
          ["messages", folder],
          {
            ...prev,
            pages: prev.pages.map((page) => ({
              ...page,
              messages: page.messages.filter((m) => m.uid !== uid),
              total_count: Math.max(0, page.total_count - 1),
            })),
          },
        );
      }
      return { prev };
    },
    onError: (_err, { folder }, context) => {
      if (context?.prev) {
        queryClient.setQueryData(["messages", folder], context.prev);
      }
    },
    onSettled: (_, _err, { folder }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", folder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

/** Update flags on multiple messages in one request instead of one per message. */
export function useBulkUpdateFlags() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      folder,
      uids,
      flags,
      add,
    }: {
      folder: string;
      uids: number[];
      flags: string[];
      add: boolean;
    }) =>
      apiPatch<BulkMessageOpResponse>(`/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/flags/bulk`, {
        uids,
        flags,
        add,
      }),
    onSuccess: (_, { folder }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", folder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

/** Move multiple messages from one folder to another in one request instead of one per message. */
export function useBulkMoveMessages() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      fromFolder,
      toFolder,
      uids,
    }: {
      fromFolder: string;
      toFolder: string;
      uids: number[];
    }) =>
      apiPost<BulkMessageOpResponse>("/messages/move/bulk", {
        from_folder: resolveFolderId(queryClient, fromFolder),
        to_folder: resolveFolderId(queryClient, toFolder),
        uids,
      }),
    onSettled: (_data, _err, { fromFolder, toFolder }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", fromFolder] });
      queryClient.invalidateQueries({ queryKey: ["messages", toFolder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

/** Permanently delete multiple messages in one request instead of one per message. */
export function useBulkDeleteMessages() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ folder, uids }: { folder: string; uids: number[] }) =>
      apiPost<BulkMessageOpResponse>(`/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/delete/bulk`, {
        uids,
      }),
    onSettled: (_data, _err, { folder }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", folder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

