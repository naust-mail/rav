"use client";

import { useInfiniteQuery } from "@tanstack/react-query";
import { apiGet } from "@/lib/api";
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
      const params = new URLSearchParams();
      if (normalizedQuery) params.set("q", normalizedQuery);
      if (folder) params.set("folder", folder);
      if (sort) params.set("sort", sort);
      params.set("limit", String(limit));
      params.set("offset", String(pageParam));

      if (filters) {
        if (filters.from) params.set("from", filters.from);
        if (filters.to) params.set("to", filters.to);
        if (filters.dateFrom) params.set("date_from", filters.dateFrom);
        if (filters.dateTo) params.set("date_to", filters.dateTo);
        if (filters.hasAttachment !== undefined)
          params.set("has_attachment", String(filters.hasAttachment));
        if (filters.folder) params.set("folder", filters.folder);
      }

      return apiGet<SearchResponse>(`/search?${params.toString()}`);
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
