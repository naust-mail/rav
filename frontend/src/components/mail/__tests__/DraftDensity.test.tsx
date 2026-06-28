import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

vi.mock("framer-motion", () => ({
  AnimatePresence: ({ children }: { children: ReactNode }) => <>{children}</>,
  motion: { div: ({ children, ...props }: React.HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div> },
}));

const { mockUiState } = vi.hoisted(() => ({
  mockUiState: {
    activeFolder: "Drafts",
    activeTagId: null as string | null,
    density: "comfortable" as "compact" | "comfortable",
    selectedMessageUid: null as number | null,
    selectedMessageUids: [] as number[],
    bulkSelectMode: false,
    keyboardNav: false,
    readingPaneVisible: true,
    effectiveAnimationMode: "off" as "rich" | "medium" | "subtle" | "off",
    selectMessage: vi.fn(),
    toggleBulkSelect: vi.fn(),
    selectAllMessages: vi.fn(),
    clearBulkSelection: vi.fn(),
    setReadingPaneVisible: vi.fn(),
    setKeyboardNav: vi.fn(),
  },
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count, estimateSize }: { count: number; estimateSize: () => number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, i) => ({ index: i, start: i * estimateSize(), size: estimateSize() })),
    getTotalSize: () => count * estimateSize(),
  }),
}));

vi.mock("@/stores/useUiStore", () => {
  const useUiStore = (selector: (state: typeof mockUiState) => unknown) => selector(mockUiState);
  useUiStore.getState = () => mockUiState;
  return { useUiStore };
});

vi.mock("@/hooks/useTags", () => ({
  useTags: () => ({ data: { tags: [] } }),
  useTagMessages: () => ({ data: { messages: [], total_count: 0 }, isLoading: false, isFetching: false, isError: false, refetch: vi.fn() }),
}));

vi.mock("@/hooks/useCompose", () => ({
  useListDrafts: () => ({
    data: {
      drafts: [
        { id: "d-1", to: "alice@example.com", subject: "Hello", updated_at: "2026-06-15T10:00:00Z" },
      ],
    },
  }),
  useGetDraft: () => ({ data: null }),
  useDeleteDraft: () => ({ mutate: vi.fn() }),
}));

vi.mock("@/stores/useComposeStore", () => ({
  useComposeStore: (selector?: (s: { openDraft: () => void; isOpen: boolean }) => unknown) => {
    const state = { openDraft: vi.fn(), isOpen: false };
    return selector ? selector(state) : state;
  },
}));

vi.mock("../BulkActionBar", () => ({
  BulkActionBar: () => null,
}));

vi.mock("@/hooks/useMessages", () => ({
  useMessages: () => ({ data: { pages: [{ messages: [], next_cursor: null }] }, isLoading: false, isFetching: false, isError: false, refetch: vi.fn(), fetchNextPage: vi.fn(), hasNextPage: false }),
  useUpdateFlags: () => ({ mutate: vi.fn() }),
}));

import { MessageList } from "../MessageList";

describe("DraftItems density", () => {
  it("renders compact row (h-9) in compact density", () => {
    mockUiState.density = "compact";
    render(<MessageList />);
    const rows = document.querySelectorAll('[role="row"]');
    expect(rows.length).toBeGreaterThan(0);
    // Compact draft row has h-9 class
    const compact = Array.from(rows).find((r) => r.className.includes("h-9"));
    expect(compact).toBeTruthy();
  });

  it("renders comfortable row (h-16) in comfortable density", () => {
    mockUiState.density = "comfortable";
    render(<MessageList />);
    const rows = document.querySelectorAll('[role="row"]');
    expect(rows.length).toBeGreaterThan(0);
    const comfortable = Array.from(rows).find((r) => r.className.includes("h-16"));
    expect(comfortable).toBeTruthy();
  });

  it("compact row shows recipient, separator dot, and subject on one row", () => {
    mockUiState.density = "compact";
    render(<MessageList />);
    expect(screen.getByText("alice@example.com")).toBeTruthy();
    expect(screen.getByText("Hello")).toBeTruthy();
    // Separator dot rendered as middot entity
    const dots = document.querySelectorAll('[role="row"] .text-muted-foreground\\/50');
    expect(dots.length).toBeGreaterThan(0);
  });
});
