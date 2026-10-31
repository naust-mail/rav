"use client";

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { apiPost } from "@/lib/api";
import type { ChangePasswordResponse } from "@/types/password";

export function useChangePassword() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: { current_password: string; new_password: string }) =>
      apiPost<ChangePasswordResponse>("/auth/password", data),
    onSuccess: () => {
      // Passkeys are removed by a password change.
      queryClient.invalidateQueries({ queryKey: ["mfa"] });
    },
  });
}
