"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import type { InfiniteData } from "@tanstack/react-query";
import { apiGet, apiPost, apiPatch, apiDelete } from "@/lib/api";
import { useWsStatus } from "@/lib/ws-context";
import type { FoldersResponse } from "@/types/folder";
import type { MessagesResponse } from "@/types/message";

const PREVIEW_PER_PAGE = 20;

export function useFolders() {
  const queryClient = useQueryClient();
  const { status } = useWsStatus();
  return useQuery({
    queryKey: ["folders"],
    queryFn: async () => {
      const data = await apiGet<FoldersResponse>("/folders");
      // Seed each folder's message cache with the preview data so clicking a
      // folder shows content immediately without a separate round trip.
      for (const folder of data.folders) {
        if (folder.recent_messages.length === 0) continue;
        queryClient.setQueryData<InfiniteData<MessagesResponse>>(
          ["messages", folder.name],
          (existing) => {
            // Don't overwrite a cache entry that's already been populated by
            // a real messages fetch (which may have fresher or more data).
            if (existing) return existing;
            return {
              pages: [
                {
                  messages: folder.recent_messages,
                  total_count: folder.total_count,
                  page: 0,
                  per_page: PREVIEW_PER_PAGE,
                },
              ],
              pageParams: [0],
            };
          },
        );
      }
      return data;
    },
    refetchInterval: status === "connected" ? false : 30_000,
  });
}

export function useCreateFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name }: { name: string }) =>
      apiPost("/folders", { name }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["folders"] }),
  });
}

export function useRenameFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, newName }: { name: string; newName: string }) =>
      apiPatch(`/folders/${encodeURIComponent(name)}`, { new_name: newName }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["folders"] }),
  });
}

export function useDeleteFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name }: { name: string }) =>
      apiDelete(`/folders/${encodeURIComponent(name)}`),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["folders"] }),
  });
}

export function useMarkAllRead() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ folder }: { folder: string }) =>
      apiPost(`/folders/${encodeURIComponent(folder)}/mark-all-read`, {}),
    onSuccess: (_data, { folder }) => {
      queryClient.invalidateQueries({ queryKey: ["messages", folder] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}
