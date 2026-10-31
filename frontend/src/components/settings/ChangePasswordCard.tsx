"use client";

import { useState } from "react";
import { KeyRound, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { useChangePassword } from "@/hooks/usePassword";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface ChangePasswordCardProps {
  /** Enrolled passkeys; they are removed by a password change. */
  passkeyCount: number;
  /** Passkey-only accounts must turn that off first (the server refuses otherwise). */
  passkeyOnly: boolean;
}

export function ChangePasswordCard({ passkeyCount, passkeyOnly }: ChangePasswordCardProps) {
  const change = useChangePassword();
  const [open, setOpen] = useState(false);
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirmNext, setConfirmNext] = useState("");
  const [error, setError] = useState<string | null>(null);

  const mismatch = confirmNext.length > 0 && next !== confirmNext;
  const canSubmit = current.length > 0 && next.length > 0 && next === confirmNext && !change.isPending;

  function reset() {
    setOpen(false);
    setCurrent("");
    setNext("");
    setConfirmNext("");
    setError(null);
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setError(null);
    try {
      const result = await change.mutateAsync({ current_password: current, new_password: next });
      reset();
      toast.success("Password changed");
      if (!result.verified) {
        toast.warning("The mail server does not accept the new password yet. If signing in fails, try again in a few minutes.");
      }
      if (result.passkeysRemoved > 0) {
        toast.info(
          result.passkeysRemoved === 1
            ? "Your passkey was removed. Add it again to keep signing in with it."
            : `Your ${result.passkeysRemoved} passkeys were removed. Add them again to keep signing in with them.`,
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "The password could not be changed.");
    }
  }

  return (
    <div className="rounded-lg border border-border p-4">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 rounded-md p-1.5 bg-muted text-muted-foreground">
          <KeyRound className="size-4" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="text-sm font-medium">Password</div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Change the password of your mailbox. It is also used by your other mail apps.
          </p>
        </div>
      </div>

      {passkeyOnly ? (
        <p className="mt-4 text-xs text-muted-foreground">
          Turn off passkey-only sign-in below before changing your password.
        </p>
      ) : !open ? (
        <div className="mt-4">
          <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
            Change password
          </Button>
        </div>
      ) : (
        <form onSubmit={handleSubmit} className="mt-4 space-y-3 max-w-sm">
          <div className="space-y-1.5">
            <Label htmlFor="current-password">Current password</Label>
            <Input
              id="current-password"
              type="password"
              autoComplete="current-password"
              autoFocus
              value={current}
              onChange={(e) => setCurrent(e.target.value)}
              disabled={change.isPending}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="new-password">New password</Label>
            <Input
              id="new-password"
              type="password"
              autoComplete="new-password"
              value={next}
              onChange={(e) => setNext(e.target.value)}
              disabled={change.isPending}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="confirm-new-password">Confirm new password</Label>
            <Input
              id="confirm-new-password"
              type="password"
              autoComplete="new-password"
              value={confirmNext}
              onChange={(e) => setConfirmNext(e.target.value)}
              disabled={change.isPending}
              aria-invalid={mismatch}
            />
            {mismatch && <p className="text-xs text-destructive">The passwords do not match.</p>}
          </div>

          {passkeyCount > 0 && (
            <p className="text-xs text-muted-foreground">
              {passkeyCount === 1
                ? "Your passkey is tied to your current password and will be removed. You can add it again afterwards."
                : `Your ${passkeyCount} passkeys are tied to your current password and will be removed. You can add them again afterwards.`}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            Other browsers signed in to this mailbox will be signed out.
          </p>

          {error && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}

          <div className="flex gap-2">
            <Button type="submit" size="sm" disabled={!canSubmit}>
              {change.isPending && <Loader2 className="size-3.5 animate-spin" />}
              Change password
            </Button>
            <Button type="button" variant="ghost" size="sm" onClick={reset} disabled={change.isPending}>
              Cancel
            </Button>
          </div>
        </form>
      )}
    </div>
  );
}
