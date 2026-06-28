"use client";

import { useMutation } from "@tanstack/react-query";
import { apiPost } from "@/lib/api";

type SpamReportResult = { trained: boolean };

export function useReportSpam() {
  return useMutation({
    mutationFn: ({ folder, uid }: { folder: string; uid: number }) =>
      apiPost<SpamReportResult>(
        `/messages/${encodeURIComponent(folder)}/${uid}/report-spam`,
        {},
      ),
  });
}

export function useReportHam() {
  return useMutation({
    mutationFn: ({ folder, uid }: { folder: string; uid: number }) =>
      apiPost<SpamReportResult>(
        `/messages/${encodeURIComponent(folder)}/${uid}/report-ham`,
        {},
      ),
  });
}
