"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPut } from "@/lib/api";
import type { VacationResponder, UpdateVacationResponder } from "@/types/vacation";

export function useVacation() {
  return useQuery({
    queryKey: ["vacation"],
    queryFn: () => apiGet<VacationResponder>("/settings/vacation"),
  });
}

export function useUpdateVacation() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: UpdateVacationResponder) =>
      apiPut<VacationResponder>("/settings/vacation", data as Record<string, unknown>),
    onSuccess: (result) => {
      queryClient.setQueryData(["vacation"], result);
    },
  });
}
