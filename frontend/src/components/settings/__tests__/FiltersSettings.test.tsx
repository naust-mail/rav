import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { FiltersSettings } from "../FiltersSettings";
import type { FilterRule } from "@/types/filter";

// ---------------------------------------------------------------------------
// Hook mocks
// ---------------------------------------------------------------------------

const {
  mockMutateUpdate,
  mockMutateDelete,
  mockMutateReorder,
  mockMutateApply,
  mockUseFilters,
  mockUseUpdateFilter,
  mockUseDeleteFilter,
  mockUseReorderFilters,
  mockUseApplyFilters,
} = vi.hoisted(() => ({
  mockMutateUpdate: vi.fn(),
  mockMutateDelete: vi.fn(),
  mockMutateReorder: vi.fn(),
  mockMutateApply: vi.fn(),
  mockUseFilters: vi.fn(),
  mockUseUpdateFilter: vi.fn(),
  mockUseDeleteFilter: vi.fn(),
  mockUseReorderFilters: vi.fn(),
  mockUseApplyFilters: vi.fn(),
}));

vi.mock("@/hooks/useFilters", () => ({
  useFilters: mockUseFilters,
  useCreateFilter: () => ({ mutate: vi.fn(), isPending: false }),
  useUpdateFilter: mockUseUpdateFilter,
  useDeleteFilter: mockUseDeleteFilter,
  useReorderFilters: mockUseReorderFilters,
  useApplyFilters: mockUseApplyFilters,
}));

vi.mock("@/hooks/useFolders", () => ({
  useFolders: () => ({ data: { folders: [{ name: "Junk", recent_messages: [] }, { name: "Archive", recent_messages: [] }] } }),
}));

vi.mock("@/hooks/useTags", () => ({
  useTags: () => ({ data: { tags: [] } }),
}));

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}));

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const ruleA: FilterRule = {
  id: "rule-a",
  name: "Spam blocker",
  enabled: true,
  priority: 0,
  match_mode: "all",
  conditions: [{ field: "from", op: "contains", value: "spammer" }],
  actions: [{ action_type: "move", action_value: "Junk" }],
  stop_processing: false,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

const ruleB: FilterRule = {
  id: "rule-b",
  name: "Newsletter archiver",
  enabled: false,
  priority: 1,
  match_mode: "any",
  conditions: [
    { field: "from", op: "ends_with", value: "@news.com" },
    { field: "subject", op: "starts_with", value: "[news]" },
  ],
  actions: [
    { action_type: "mark_read", action_value: null },
    { action_type: "move", action_value: "Archive" },
  ],
  stop_processing: true,
  created_at: "2026-01-02T00:00:00Z",
  updated_at: "2026-01-02T00:00:00Z",
};

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

function setup(rules: FilterRule[] = [], isLoading = false) {
  mockUseFilters.mockReturnValue({ data: { rules }, isLoading });
  mockUseUpdateFilter.mockReturnValue({ mutate: mockMutateUpdate });
  mockUseDeleteFilter.mockReturnValue({ mutate: mockMutateDelete, isPending: false });
  mockUseReorderFilters.mockReturnValue({ mutate: mockMutateReorder, isPending: false });
  mockUseApplyFilters.mockReturnValue({ mutate: mockMutateApply, isPending: false });
}

beforeEach(() => {
  vi.clearAllMocks();
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("FiltersSettings", () => {
  it("shows spinner while loading", () => {
    setup([], true);
    render(<FiltersSettings />);
    expect(screen.getByText("Loading filters...")).toBeDefined();
  });

  it("shows empty state when no rules exist", () => {
    setup([]);
    render(<FiltersSettings />);
    expect(screen.getByText("No filter rules yet.")).toBeDefined();
    expect(screen.getByText("Create your first rule")).toBeDefined();
  });

  it("does not show Apply button when there are no rules", () => {
    setup([]);
    render(<FiltersSettings />);
    expect(screen.queryByText("Apply to inbox")).toBeNull();
  });

  it("renders rule names and descriptions", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    expect(screen.getByText("Spam blocker")).toBeDefined();
    expect(screen.getByText("Newsletter archiver")).toBeDefined();
  });

  it("renders the 'stops' badge only for stop_processing rules", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    const badges = screen.getAllByText("stops");
    expect(badges).toHaveLength(1);
  });

  it("renders enabled checkboxes reflecting rule state", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    const checkboxes = screen.getAllByRole("checkbox");
    // ruleA enabled=true, ruleB enabled=false
    expect((checkboxes[0] as HTMLInputElement).checked).toBe(true);
    expect((checkboxes[1] as HTMLInputElement).checked).toBe(false);
  });

  it("calls updateFilter with toggled enabled when checkbox is clicked", () => {
    setup([ruleA]);
    render(<FiltersSettings />);
    const checkbox = screen.getByRole("checkbox");
    fireEvent.click(checkbox);
    expect(mockMutateUpdate).toHaveBeenCalledWith(
      { id: "rule-a", data: { enabled: false } },
      expect.objectContaining({ onError: expect.any(Function) }),
    );
  });

  it("calls deleteFilter with rule id when delete button is clicked", () => {
    setup([ruleA]);
    render(<FiltersSettings />);
    fireEvent.click(screen.getByLabelText("Delete rule"));
    expect(mockMutateDelete).toHaveBeenCalledWith(
      "rule-a",
      expect.objectContaining({ onSuccess: expect.any(Function), onError: expect.any(Function) }),
    );
  });

  it("opens the dialog when New rule button is clicked", () => {
    setup([]);
    render(<FiltersSettings />);
    fireEvent.click(screen.getByText("New rule"));
    expect(screen.getByText("New filter rule")).toBeDefined();
  });

  it("opens edit dialog when pencil button is clicked", () => {
    setup([ruleA]);
    render(<FiltersSettings />);
    fireEvent.click(screen.getByLabelText("Edit rule"));
    expect(screen.getByText("Edit filter rule")).toBeDefined();
  });

  it("shows Apply to inbox button when rules exist", () => {
    setup([ruleA]);
    render(<FiltersSettings />);
    expect(screen.getByText("Apply to inbox")).toBeDefined();
  });

  it("calls applyFilters when Apply to inbox is clicked", () => {
    setup([ruleA]);
    render(<FiltersSettings />);
    fireEvent.click(screen.getByText("Apply to inbox"));
    expect(mockMutateApply).toHaveBeenCalledWith(
      undefined,
      expect.objectContaining({ onSuccess: expect.any(Function), onError: expect.any(Function) }),
    );
  });

  it("calls reorderFilters with swapped ids when up arrow is clicked", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    // ruleB is at index 1; clicking its up arrow should move it before ruleA
    const upButtons = screen.getAllByLabelText("Move rule up");
    fireEvent.click(upButtons[1]); // second rule's up button
    expect(mockMutateReorder).toHaveBeenCalledWith(
      { ids: ["rule-b", "rule-a"] },
      expect.objectContaining({ onError: expect.any(Function) }),
    );
  });

  it("calls reorderFilters with swapped ids when down arrow is clicked", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    const downButtons = screen.getAllByLabelText("Move rule down");
    fireEvent.click(downButtons[0]); // first rule's down button
    expect(mockMutateReorder).toHaveBeenCalledWith(
      { ids: ["rule-b", "rule-a"] },
      expect.objectContaining({ onError: expect.any(Function) }),
    );
  });

  it("disables the first up button and last down button", () => {
    setup([ruleA, ruleB]);
    render(<FiltersSettings />);
    const upButtons = screen.getAllByLabelText("Move rule up");
    const downButtons = screen.getAllByLabelText("Move rule down");
    expect((upButtons[0] as HTMLButtonElement).disabled).toBe(true);
    expect((upButtons[1] as HTMLButtonElement).disabled).toBe(false);
    expect((downButtons[0] as HTMLButtonElement).disabled).toBe(false);
    expect((downButtons[1] as HTMLButtonElement).disabled).toBe(true);
  });

  describe("rule description rendering", () => {
    it("uses AND separator for multi-condition match_mode=all rules", () => {
      const multiAndRule: FilterRule = {
        ...ruleA,
        id: "rule-multi",
        conditions: [
          { field: "from", op: "contains", value: "spammer" },
          { field: "subject", op: "contains", value: "promo" },
        ],
      };
      setup([multiAndRule]);
      render(<FiltersSettings />);
      const desc = screen.getByText(/spammer/);
      expect(desc.textContent).toContain("AND");
    });

    it("uses OR separator for match_mode=any", () => {
      setup([ruleB]);
      render(<FiltersSettings />);
      const desc = screen.getByText(/@news\.com/);
      expect(desc.textContent).toContain("OR");
    });

    it("shows all actions joined by comma", () => {
      setup([ruleB]);
      render(<FiltersSettings />);
      const desc = screen.getByText(/Mark as read/);
      expect(desc.textContent).toContain("Mark as read");
      expect(desc.textContent).toContain("Move to folder");
    });
  });
});
