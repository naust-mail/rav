"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, apiPut, apiDelete } from "@/lib/api";
import type { FilterRule, FiltersResponse, CreateFilterRule, UpdateFilterRule } from "@/types/filter";

export function useFilters() {
  return useQuery({
    queryKey: ["filters"],
    queryFn: () => apiGet<FiltersResponse>("/filters"),
  });
}

export function useCreateFilter() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: CreateFilterRule) =>
      apiPost<FilterRule>("/filters", data as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["filters"] });
    },
  });
}

export function useUpdateFilter() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: UpdateFilterRule }) =>
      apiPut<FilterRule>(`/filters/${id}`, data as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["filters"] });
    },
  });
}

export function useDeleteFilter() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiDelete(`/filters/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["filters"] });
    },
  });
}
