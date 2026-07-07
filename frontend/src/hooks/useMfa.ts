"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPost, apiDelete, apiPut } from "@/lib/api";
import type {
  MfaStatus,
  TotpSetupResponse,
  TotpToggleResponse,
  PasskeyListResponse,
  PasskeyOnlyResponse,
} from "@/types/mfa";

export function useMfaStatus() {
  return useQuery({
    queryKey: ["mfa", "status"],
    queryFn: () => apiGet<MfaStatus>("/mfa/status"),
  });
}

export function useTotpSetup() {
  return useMutation({
    mutationFn: () => apiPost<TotpSetupResponse>("/mfa/totp/setup", {}),
  });
}

export function useTotpConfirm() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: { secret: string; code: string }) =>
      apiPost<TotpToggleResponse>("/mfa/totp/confirm", data as Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mfa", "status"] });
    },
  });
}

export function useTotpDelete() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (code: string) => apiDelete<TotpToggleResponse>("/mfa/totp", { code }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mfa", "status"] });
    },
  });
}

export function usePasskeyList() {
  return useQuery({
    queryKey: ["mfa", "passkeys"],
    queryFn: () => apiGet<PasskeyListResponse>("/mfa/passkeys"),
  });
}

export function usePasskeyDelete() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiDelete<{ deleted: boolean }>(`/mfa/passkeys/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mfa", "passkeys"] });
      queryClient.invalidateQueries({ queryKey: ["mfa", "status"] });
    },
  });
}

export function usePasskeySetOnly() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (enabled: boolean) =>
      apiPut<PasskeyOnlyResponse>("/mfa/settings/passkey-only", { enabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mfa", "status"] });
    },
  });
}
