import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ChangePasswordCard } from "../ChangePasswordCard";

const { mockMutateAsync, mockToast } = vi.hoisted(() => ({
  mockMutateAsync: vi.fn(),
  mockToast: { success: vi.fn(), warning: vi.fn(), info: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/usePassword", () => ({
  useChangePassword: () => ({ mutateAsync: mockMutateAsync, isPending: false }),
}));

vi.mock("sonner", () => ({ toast: mockToast }));

function fillForm(current: string, next: string, confirm: string) {
  fireEvent.change(screen.getByLabelText("Current password"), { target: { value: current } });
  fireEvent.change(screen.getByLabelText("New password"), { target: { value: next } });
  fireEvent.change(screen.getByLabelText("Confirm new password"), { target: { value: confirm } });
}

function submitButton() {
  return screen.getAllByRole("button", { name: "Change password" }).at(-1)!;
}

describe("ChangePasswordCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("asks to turn off passkey-only sign-in instead of showing the form", () => {
    render(<ChangePasswordCard passkeyCount={1} passkeyOnly />);
    expect(screen.getByText(/Turn off passkey-only sign-in/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Change password" })).toBeNull();
  });

  it("keeps submit disabled while the new passwords differ", () => {
    render(<ChangePasswordCard passkeyCount={0} passkeyOnly={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));
    fillForm("old-password", "new-password", "new-passw0rd");

    expect(screen.getByText("The passwords do not match.")).toBeTruthy();
    expect((submitButton() as HTMLButtonElement).disabled).toBe(true);
  });

  it("warns that passkeys will be removed", () => {
    render(<ChangePasswordCard passkeyCount={2} passkeyOnly={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));
    expect(screen.getByText(/Your 2 passkeys are tied to your current password/)).toBeTruthy();
  });

  it("submits the passwords and reports success", async () => {
    mockMutateAsync.mockResolvedValue({
      status: "changed",
      verified: true,
      passkeysRemoved: 1,
      otherSessionsSignedOut: 0,
    });
    render(<ChangePasswordCard passkeyCount={1} passkeyOnly={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));
    fillForm("old-password", "new-password", "new-password");
    fireEvent.click(submitButton());

    await waitFor(() => expect(mockToast.success).toHaveBeenCalledWith("Password changed"));
    expect(mockMutateAsync).toHaveBeenCalledWith({
      current_password: "old-password",
      new_password: "new-password",
    });
    expect(mockToast.info).toHaveBeenCalled();
    expect(mockToast.warning).not.toHaveBeenCalled();
    // The form closes and forgets the passwords.
    expect(screen.queryByLabelText("Current password")).toBeNull();
  });

  it("shows the server's error and keeps the form open", async () => {
    mockMutateAsync.mockRejectedValue(new Error("The current password is incorrect"));
    render(<ChangePasswordCard passkeyCount={0} passkeyOnly={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Change password" }));
    fillForm("wrong", "new-password", "new-password");
    fireEvent.click(submitButton());

    expect((await screen.findByRole("alert")).textContent).toBe("The current password is incorrect");
    expect(screen.getByLabelText("Current password")).toBeTruthy();
  });
});
