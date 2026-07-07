"use client";

import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { InfiniteData } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";
import type { MessagesResponse } from "@/types/message";

const { mockApiGet } = vi.hoisted(() => ({
  mockApiGet: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  apiGet: mockApiGet,
}));

vi.mock("@/lib/ws-context", () => ({
  useWsStatus: () => ({ status: "connected", failCount: 0 }),
}));

import { useFolders } from "../useFolders";

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return {
    queryClient,
    wrapper: function Wrapper({ children }: { children: ReactNode }) {
      return createElement(QueryClientProvider, { client: queryClient }, children);
    },
  };
}

const mockMessage = {
  uid: 1,
  folder: "INBOX",
  subject: "Hello",
  from_address: "a@b.com",
  from_name: "Alice",
  to_addresses: "[]",
  cc_addresses: "[]",
  date: "2024-01-01",
  flags: "",
  size: 100,
  has_attachments: false,
  snippet: "Hello there",
  reaction: null,
  tags: [],
  thread_count: 1,
  unread_count: 1,
};

describe("useFolders", () => {
  afterEach(() => {
    mockApiGet.mockReset();
  });

  it("seeds the message cache for each folder from recent_messages", async () => {
    mockApiGet.mockResolvedValue({
      folders: [
        {
          name: "INBOX",
          delimiter: "/",
          attributes: [],
          is_subscribed: true,
          total_count: 1,
          unread_count: 1,
          recent_messages: [mockMessage],
        },
      ],
    });

    const { queryClient, wrapper } = createWrapper();
    const { result } = renderHook(() => useFolders(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = queryClient.getQueryData<InfiniteData<MessagesResponse>>(["messages", "INBOX"]);
    expect(cached).toBeDefined();
    expect(cached!.pages[0].messages[0].subject).toBe("Hello");
    expect(cached!.pages[0].total_count).toBe(1);
  });

  it("does not overwrite an existing message cache entry", async () => {
    mockApiGet.mockResolvedValue({
      folders: [
        {
          name: "INBOX",
          delimiter: "/",
          attributes: [],
          is_subscribed: true,
          total_count: 5,
          unread_count: 2,
          recent_messages: [{ ...mockMessage, subject: "Newer from folders" }],
        },
      ],
    });

    const { queryClient, wrapper } = createWrapper();

    // Pre-populate the cache as if a real messages fetch already ran.
    const existing: InfiniteData<MessagesResponse> = {
      pages: [{ messages: [{ ...mockMessage, subject: "Already cached" }], total_count: 5, page: 0, per_page: 50 }],
      pageParams: [0],
    };
    queryClient.setQueryData(["messages", "INBOX"], existing);

    renderHook(() => useFolders(), { wrapper });

    await waitFor(() =>
      expect(mockApiGet).toHaveBeenCalledTimes(1),
    );

    const cached = queryClient.getQueryData<InfiniteData<MessagesResponse>>(["messages", "INBOX"]);
    expect(cached!.pages[0].messages[0].subject).toBe("Already cached");
  });

  it("skips folders with no recent messages", async () => {
    mockApiGet.mockResolvedValue({
      folders: [
        {
          name: "Sent",
          delimiter: "/",
          attributes: [],
          is_subscribed: true,
          total_count: 0,
          unread_count: 0,
          recent_messages: [],
        },
      ],
    });

    const { queryClient, wrapper } = createWrapper();
    const { result } = renderHook(() => useFolders(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    const cached = queryClient.getQueryData(["messages", "Sent"]);
    expect(cached).toBeUndefined();
  });
});
