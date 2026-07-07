"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, apiDelete, apiPut } from "@/lib/api";
import type { PgpKeySummary, PgpKeyRecord, HealthResponse, ServerCapability } from "@/types/pgp";

interface PgpKeysResponse {
  keys: PgpKeySummary[];
}

interface StoreKeyParams {
  id: string;
  fingerprint: string;
  public_key: string;
  private_key_enc: string;
  identity_id?: number | null;
}

interface AssignIdentityParams {
  id: string;
  identity_id: number | null;
}

/** Returns the set of capabilities the server has enabled. Fetched once on load, no auth required. */
export function useServerCapability(cap: ServerCapability): boolean {
  const { data } = useQuery({
    queryKey: ["health"],
    queryFn: () => apiGet<HealthResponse>("/health"),
    staleTime: Infinity,
  });
  return data?.capabilities.includes(cap) ?? true;
}

export function usePgpKeys() {
  return useQuery({
    queryKey: ["pgp-keys"],
    queryFn: () => apiGet<PgpKeysResponse>("/pgp/keys").then((r) => r.keys),
  });
}

export function usePgpKey(id: string) {
  return useQuery({
    queryKey: ["pgp-keys", id],
    queryFn: () => apiGet<PgpKeyRecord>(`/pgp/keys/${id}`),
    enabled: id.length > 0,
  });
}

export function useStorePgpKey() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (params: StoreKeyParams) =>
      apiPost<PgpKeyRecord>("/pgp/keys", params as unknown as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pgp-keys"] });
    },
  });
}

export function useDeletePgpKey() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiDelete<{ status: string }>(`/pgp/keys/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pgp-keys"] });
    },
  });
}

export function useAssignPgpIdentity() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, identity_id }: AssignIdentityParams) =>
      apiPut<{ status: string }>(`/pgp/keys/${id}/identity`, {
        identity_id: identity_id as unknown as Record<string, unknown>,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["pgp-keys"] });
    },
  });
}
