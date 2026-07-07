"use client";

import { useInfiniteQuery } from "@tanstack/react-query";
import { apiPost } from "@/lib/api";
import {
  isValidCommittedSearch,
  normalizeSearchQuery,
} from "@/lib/search-parser";
import type { SearchResponse } from "@/types/message";

export interface SearchFilters {
  from?: string;
  to?: string;
  dateFrom?: string;
  dateTo?: string;
  hasAttachment?: boolean;
  folder?: string;
}

const FIRST_PAGE_SIZE = 200;
const NEXT_PAGE_SIZE = 100;

export function useSearch(
  query: string,
  folder?: string,
  sort: "date_desc" | "date_asc" = "date_desc",
  filters?: SearchFilters,
) {
  const normalizedQuery = normalizeSearchQuery(query);
  const hasValidCommittedQuery = isValidCommittedSearch(normalizedQuery);

  return useInfiniteQuery({
    queryKey: ["search", normalizedQuery, folder, sort, filters],
    queryFn: ({ pageParam }) => {
      const limit = pageParam === 0 ? FIRST_PAGE_SIZE : NEXT_PAGE_SIZE;
      const body: Record<string, unknown> = {
        q: normalizedQuery,
        sort,
        limit,
        offset: pageParam,
      };
      if (folder) body.folder = folder;
      if (filters) {
        if (filters.from) body.from = filters.from;
        if (filters.to) body.to = filters.to;
        if (filters.dateFrom) body.date_from = filters.dateFrom;
        if (filters.dateTo) body.date_to = filters.dateTo;
        if (filters.hasAttachment !== undefined) body.has_attachment = filters.hasAttachment;
        if (filters.folder) body.folder = filters.folder;
      }
      return apiPost<SearchResponse>(`/search`, body);
    },
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) => {
      const loaded = allPages.reduce((sum, p) => sum + p.results.length, 0);
      return loaded < lastPage.total_count ? loaded : undefined;
    },
    enabled: hasValidCommittedQuery,
    placeholderData: (prev) => prev,
  });
}
