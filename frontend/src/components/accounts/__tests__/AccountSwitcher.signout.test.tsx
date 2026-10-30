import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";

vi.mock("framer-motion", async () => {
  function AnimatePresence({ children }: { children: ReactNode }) {
    return <>{children}</>;
  }

  return {
    AnimatePresence,
    motion: {
      div: ({ children, ...props }: React.HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div>,
    },
  };
});

const { mockAuthState, mockApiPost } = vi.hoisted(() => ({
  mockAuthState: {
    accounts: [] as { id: string; email: string; imapHost: string }[],
    activeAccountId: "a1",
    setActiveAccount: vi.fn(),
    removeAccount: vi.fn(),
    setAccounts: vi.fn(),
  },
  mockApiPost: vi.fn(async () => ({})),
}));

vi.mock("@/stores/useAuthStore", () => ({
  useAuthStore: (selector: (state: typeof mockAuthState) => unknown) => selector(mockAuthState),
}));

vi.mock("@/stores/useUiStore", () => ({
  useUiStore: (selector: (state: { effectiveAnimationMode: string }) => unknown) =>
    selector({ effectiveAnimationMode: "off" }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ clear: vi.fn() }),
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
}));

vi.mock("@/lib/api", () => ({
  apiDelete: vi.fn(async () => ({})),
  apiPost: mockApiPost,
}));

vi.mock("../AddAccountModal", () => ({
  AddAccountModal: () => null,
}));

import { AccountSwitcher } from "../AccountSwitcher";

const admin = { id: "a1", email: "admin@example.com", imapHost: "mail.example.com" };
const ops = { id: "a2", email: "ops@example.com", imapHost: "imap.ops.example.com" };

describe("AccountSwitcher sign out", () => {
  beforeEach(() => {
    mockApiPost.mockClear();
  });

  it("offers sign out with a single account", async () => {
    mockAuthState.accounts = [admin];
    render(<AccountSwitcher />);

    fireEvent.click(screen.getByRole("button", { name: /admin@example.com/i }));
    expect(screen.queryByText("Sign out all accounts")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(mockApiPost).toHaveBeenCalledWith("/auth/logout", {}));
  });

  it("offers sign out of all accounts with several accounts", () => {
    mockAuthState.accounts = [admin, ops];
    render(<AccountSwitcher />);

    fireEvent.click(screen.getByRole("button", { name: /admin@example.com/i }));
    expect(screen.getByRole("button", { name: "Sign out all accounts" })).toBeTruthy();
  });
});
