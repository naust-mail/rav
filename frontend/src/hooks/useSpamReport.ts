"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { apiPost } from "@/lib/api";
import { resolveFolderId } from "@/lib/folders";

type SpamReportResult = { trained: boolean };

export function useReportSpam() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ folder, uid }: { folder: string; uid: number }) =>
      apiPost<SpamReportResult>(
        `/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/${uid}/report-spam`,
        {},
      ),
  });
}

export function useReportHam() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ folder, uid }: { folder: string; uid: number }) =>
      apiPost<SpamReportResult>(
        `/messages/${encodeURIComponent(resolveFolderId(queryClient, folder))}/${uid}/report-ham`,
        {},
      ),
  });
}
