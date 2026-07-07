"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPut, apiDelete } from "@/lib/api";
import type { CalendarSticker, CalendarStickersResponse, PutStickerRequest } from "@/types/sticker";

export function useCalendarStickers(from: string, to: string) {
  return useQuery({
    queryKey: ["calendar-stickers", from, to],
    queryFn: () =>
      apiGet<CalendarStickersResponse>(
        `/calendar/stickers?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      ),
    enabled: !!from && !!to,
    select: (data) => {
      // Index by date for O(1) lookup in the calendar grid.
      const byDate = new Map<string, CalendarSticker>();
      for (const s of data.stickers) byDate.set(s.date, s);
      return byDate;
    },
  });
}

export function usePutSticker() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ date, sticker_id }: { date: string; sticker_id: string }) =>
      apiPut<CalendarSticker>(`/calendar/stickers/${date}`, { sticker_id } as unknown as PutStickerRequest & Record<string, unknown>),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["calendar-stickers"] });
    },
  });
}

export function useDeleteSticker() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (date: string) => apiDelete(`/calendar/stickers/${date}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["calendar-stickers"] });
    },
  });
}
