"use client";

import { useState, type FormEvent, useEffect, useRef, useMemo } from "react";
import { Dialog } from "radix-ui";
import { AnimatePresence } from "framer-motion";
import { X, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { apiPost, fetchAccounts } from "@/lib/api";
import { useAuthStore } from "@/stores/useAuthStore";
import { useQueryClient } from "@tanstack/react-query";
import { useUiStore } from "@/stores/useUiStore";
import type { UiState } from "@/stores/useUiStore";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";

function getCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  const match = document.cookie.match(new RegExp(`(^| )${name}=([^;]+)`));
  return match ? match[2] : null;
}

interface AddAccountModalProps {
  open: boolean;
  onClose: () => void;
}

export function AddAccountModal({ open, onClose }: AddAccountModalProps) {
  const setAccounts = useAuthStore((s) => s.setAccounts);
  const setActiveAccount = useAuthStore((s) => s.setActiveAccount);
  const queryClient = useQueryClient();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [remember, setRemember] = useState(false);
  const [loading, setLoading] = useState(false);
  const effectiveAnimationMode = useUiStore((s: UiState) => s.effectiveAnimationMode);
  const overlayMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const contentMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const emailInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setEmail("");
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setPassword("");
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setError(null);
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setRemember(false);
      const timer = setTimeout(() => emailInputRef.current?.focus(), 50);
      return () => clearTimeout(timer);
    }
  }, [open]);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError(null);

    const browserId = getCookie("oxi_browser");

    const payload: Record<string, unknown> = {
      email,
      password,
      remember,
    };

    if (browserId) {
      payload.browser_id = browserId;
    }

    try {
      const response = await apiPost<{
        account: { id: string; email: string; imapHost: string; smtpHost: string };
      }>("/auth/login", payload);

      const accountsData = await fetchAccounts();
      setAccounts(accountsData.accounts);
      setActiveAccount(response.account.id);
      queryClient.clear();
      onClose();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "An unexpected error occurred",
      );
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal forceMount>
        <AnimatePresence>
          {open ? (
            <>
              <Dialog.Overlay asChild>
                <AnimatedDiv
                  key="add-account-overlay"
                  data-testid="add-account-overlay-transition"
                  variants={overlayMotionProps}
                  initial="initial"
                  animate="animate"
                  exit="exit"
                  className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm"
                />
              </Dialog.Overlay>
              <Dialog.Content asChild>
                <AnimatedDiv
                  data-testid="add-account-content-transition"
                  variants={contentMotionProps}
                  initial="initial"
                  animate="animate"
                  exit="exit"
                  className="fixed left-1/2 top-1/2 z-50 w-full max-w-md -translate-x-1/2 -translate-y-1/2 rounded-xl border border-border bg-background p-6 shadow-2xl max-h-[90vh] overflow-y-auto"
                >
          <Dialog.Title className="text-lg font-semibold mb-1">
            Add account
          </Dialog.Title>
          <Dialog.Description className="text-sm text-muted-foreground mb-4">
            Sign in to another email account
          </Dialog.Description>
          <Dialog.Close asChild>
            <button
              type="button"
              className="absolute right-4 top-4 rounded-md p-1 text-muted-foreground hover:text-foreground transition-colors"
              aria-label="Close"
            >
              <X className="size-4" />
            </button>
          </Dialog.Close>

          {error && (
            <div className="mb-4 rounded-md bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {error}
            </div>
          )}

          <form onSubmit={handleSubmit} className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label htmlFor="add-email">Email</Label>
              <Input
                ref={emailInputRef}
                id="add-email"
                type="email"
                placeholder="you@example.com"
                autoComplete="email"
                required
                disabled={loading}
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            <div className="flex flex-col gap-2">
              <Label htmlFor="add-password">Password</Label>
              <Input
                id="add-password"
                type="password"
                placeholder="Enter your password"
                autoComplete="current-password"
                required
                disabled={loading}
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            </div>
            <label htmlFor="add-remember" className="flex items-center gap-2 cursor-pointer select-none">
              <input
                id="add-remember"
                type="checkbox"
                checked={remember}
                onChange={(e) => setRemember(e.target.checked)}
                disabled={loading}
                className="size-4 rounded border-muted-foreground/40 accent-primary"
              />
              <span className="text-sm text-muted-foreground">Keep me logged in</span>
            </label>

            <div className="flex justify-end gap-2 mt-2">
              <Button
                type="button"
                variant="outline"
                onClick={onClose}
                disabled={loading}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={loading}
              >
                {loading && <Loader2 className="size-4 animate-spin" />}
                {loading ? "Signing in..." : "Sign in"}
              </Button>
            </div>
          </form>
                </AnimatedDiv>
              </Dialog.Content>
            </>
          ) : null}
        </AnimatePresence>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
