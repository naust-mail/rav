"use client";

import { useInfiniteQuery, useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, apiPostFormData, apiDelete } from "@/lib/api";
import type { Contact, ContactsResponse } from "@/types/contact";

const PAGE_SIZE = 30;

export function useContacts(search?: string) {
  return useInfiniteQuery({
    queryKey: ["contacts", search ?? ""],
    queryFn: ({ pageParam = 0 }) => {
      const params = new URLSearchParams();
      if (search) params.set("q", search);
      params.set("limit", String(PAGE_SIZE));
      params.set("offset", String(pageParam));
      return apiGet<ContactsResponse>(`/contacts?${params.toString()}`);
    },
    initialPageParam: 0,
    getNextPageParam: (lastPage, allPages) => {
      const loaded = allPages.reduce((n, p) => n + p.contacts.length, 0);
      return loaded < lastPage.total_count ? loaded : undefined;
    },
  });
}

export function useCreateContact() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: {
      email: string;
      name: string;
      company?: string;
      notes?: string;
      is_favorite?: boolean;
    }) => apiPost<Contact>("/contacts", body as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

export function useUpdateContact() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (body: {
      id: string;
      email: string;
      name: string;
      company?: string;
      notes?: string;
      is_favorite?: boolean;
      source?: string;
    }) => apiPost<Contact>("/contacts", body as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

export function useDeleteContact() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiDelete(`/contacts/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

export function useImportContacts() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (file: File) => {
      const formData = new FormData();
      formData.append("file", file);
      return apiPostFormData<{ created: number; updated: number; skipped: number }>(
        "/contacts/import",
        formData,
      );
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
    },
  });
}

export function useAutocomplete(query: string) {
  return useQuery({
    queryKey: ["contacts-autocomplete", query],
    queryFn: async () => {
      const res = await apiGet<{ suggestions: { email: string; name: string }[] }>(
        `/contacts/autocomplete?q=${encodeURIComponent(query)}&limit=10`,
      );
      return res.suggestions;
    },
    enabled: query.length >= 2,
  });
}
