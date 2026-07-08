import { renderHook, waitFor } from "@testing-library/react";
import {
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement, type ReactNode } from "react";

const { mockApiPost } = vi.hoisted(() => ({
  mockApiPost: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  apiPost: mockApiPost,
}));

import { useSearch } from "../useSearch";

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
      },
    },
  });

  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
  };
}

describe("useSearch", () => {
  afterEach(() => {
    mockApiPost.mockReset();
  });

  it("normalizes committed query before building params", async () => {
    mockApiPost.mockResolvedValue({ total_count: 0, results: [] });

    renderHook(() => useSearch("   from:alice    report   "), {
      wrapper: createWrapper(),
    });

    await waitFor(() => {
      expect(mockApiPost).toHaveBeenCalledTimes(1);
    });

    expect(mockApiPost).toHaveBeenCalledWith("/search", {
      q: "from:alice report",
      sort: "date_desc",
      limit: 200,
      offset: 0,
    });
  });

  it("does not call apiPost for empty or whitespace queries", async () => {
    mockApiPost.mockResolvedValue({ total_count: 0, results: [] });

    const { rerender } = renderHook(
      ({ query }) => useSearch(query),
      {
        wrapper: createWrapper(),
        initialProps: { query: "" },
      },
    );

    rerender({ query: "     " });

    await waitFor(() => {
      expect(mockApiPost).not.toHaveBeenCalled();
    });
  });
});
