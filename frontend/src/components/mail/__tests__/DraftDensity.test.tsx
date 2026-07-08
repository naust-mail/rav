import { render, screen, fireEvent } from "@testing-library/react";
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
  useGetDraftAttachments: () => ({ data: null, isPending: false }),
}));

const mockOpenDraft = vi.fn();

vi.mock("@/stores/useComposeStore", () => ({
  useComposeStore: (selector?: (s: { openDraft: () => void; isOpen: boolean }) => unknown) => {
    const state = { openDraft: mockOpenDraft, isOpen: false };
    return selector ? selector(state) : state;
  },
}));

vi.mock("../BulkActionBar", () => ({
  BulkActionBar: () => null,
}));

const draftMessage = {
  uid: 1,
  folder: "Drafts",
  subject: "Draft subject",
  from_address: "me@example.com",
  from_name: "Me",
  to_addresses: "alice@example.com",
  cc_addresses: "",
  date: "2026-06-15T10:00:00Z",
  flags: "\\Draft \\Seen",
  size: 100,
  has_attachments: false,
  snippet: "Draft content",
  reaction: null,
  tags: [],
  thread_count: 1,
  unread_count: 0,
};

const mockMessageDetail = {
  uid: 1,
  folder: "Drafts",
  subject: "Draft subject",
  from_address: "me@example.com",
  from_name: "Me",
  to_addresses: [{ name: null, address: "alice@example.com" }],
  cc_addresses: [],
  date: "2026-06-15T10:00:00Z",
  flags: ["\\Draft", "\\Seen"],
  html: "<p>Draft body</p>",
  text: null,
  raw_headers: "Message-ID: <test-uuid-123@draft>\r\nSubject: Draft subject\r\n",
  attachments: [],
  thread: [],
  pgp_status: null,
};

vi.mock("@/hooks/useMessages", () => ({
  useMessages: () => ({
    data: { pages: [{ messages: [draftMessage], total_count: 1, syncing: false }] },
    isLoading: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
    fetchNextPage: vi.fn(),
    hasNextPage: false,
    isFetchingNextPage: false,
  }),
  useMessage: () => ({ data: mockMessageDetail, isPending: false }),
  useMessageByMessageId: () => ({ data: undefined, isPending: false }),
  useUpdateFlags: () => ({ mutate: vi.fn() }),
}));

import { MessageList } from "../MessageList";

describe("Draft message in IMAP list", () => {
  it("renders the draft message row", () => {
    render(<MessageList />);
    expect(screen.getByText("Draft subject")).toBeTruthy();
  });

  it("clicking a draft message does not call selectMessage", () => {
    mockUiState.selectMessage = vi.fn();
    render(<MessageList />);
    const row = screen.getByText("Draft subject").closest("[data-testid]") ?? screen.getByText("Draft subject");
    fireEvent.click(row);
    expect(mockUiState.selectMessage).not.toHaveBeenCalled();
  });
});
