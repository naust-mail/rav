"use client";

import { useEffect, useRef, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { apiPost, fetchAccounts } from "@/lib/api";
import { useAuthStore } from "@/stores/useAuthStore";
import { useQueryClient } from "@tanstack/react-query";
import type {
  LoginResponse,
  LoginSuccess,
  PasskeyAssertionResponse,
  PasskeyLoginBeginResponse,
} from "@/types/mfa";

// ---------------------------------------------------------------------------
// WebAuthn helpers
// ---------------------------------------------------------------------------

function base64urlToBuffer(b64: string): ArrayBuffer {
  const padded = b64.padEnd(b64.length + ((4 - (b64.length % 4)) % 4), "=");
  const binary = atob(padded.replace(/-/g, "+").replace(/_/g, "/"));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

function bufferToBase64url(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let s = "";
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=/g, "");
}

function preparePublicKeyOptions(
  opts: PasskeyLoginBeginResponse["options"]["publicKey"],
): PublicKeyCredentialRequestOptions {
  return {
    challenge: base64urlToBuffer(opts.challenge),
    timeout: opts.timeout,
    rpId: opts.rpId,
    allowCredentials: (opts.allowCredentials ?? []).map((c) => ({
      type: c.type as PublicKeyCredentialType,
      id: base64urlToBuffer(c.id),
    })),
    userVerification:
      (opts.userVerification as UserVerificationRequirement) ?? "required",
  };
}

function serializeCredential(cred: PublicKeyCredential): PasskeyAssertionResponse {
  const assertion = cred.response as AuthenticatorAssertionResponse;
  const ext = cred.getClientExtensionResults() as {
    prf?: { results?: { first?: ArrayBuffer } };
  };

  return {
    id: cred.id,
    rawId: bufferToBase64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: bufferToBase64url(assertion.authenticatorData),
      clientDataJSON: bufferToBase64url(assertion.clientDataJSON),
      signature: bufferToBase64url(assertion.signature),
      userHandle: assertion.userHandle
        ? bufferToBase64url(assertion.userHandle)
        : null,
    },
    clientExtensionResults: {
      prf: ext.prf
        ? {
            results: ext.prf.results
              ? {
                  first: ext.prf.results.first
                    ? bufferToBase64url(ext.prf.results.first)
                    : undefined,
                }
              : undefined,
          }
        : undefined,
    },
  };
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

function getCookie(name: string): string | null {
  if (typeof document === "undefined") return null;
  const match = document.cookie.match(new RegExp(`(^| )${name}=([^;]+)`));
  return match ? match[2] : null;
}

type LoginStep = "email" | "password" | "totp";

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export default function Home() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const setAccounts = useAuthStore((s) => s.setAccounts);
  const setActiveAccount = useAuthStore((s) => s.setActiveAccount);

  const [checking, setChecking] = useState(true);
  const [step, setStep] = useState<LoginStep>("email");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [remember, setRemember] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [totpCode, setTotpCode] = useState("");
  const [supportsPasskey] = useState(
    () =>
      typeof window !== "undefined" &&
      typeof navigator.credentials !== "undefined" &&
      typeof window.PublicKeyCredential !== "undefined",
  );

  const totpInputRef = useRef<HTMLInputElement>(null);
  const abortRef = useRef<AbortController | null>(null);
  const hiddenPasswordRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let cancelled = false;
    fetchAccounts()
      .then((data) => {
        if (!cancelled) {
          if (data.accounts.length > 0) {
            router.replace("/mail");
          } else {
            setChecking(false);
          }
        }
      })
      .catch(() => {
        if (!cancelled) setChecking(false);
      });
    return () => {
      cancelled = true;
    };
  }, [router]);

  useEffect(() => {
    if (step === "totp" && totpInputRef.current) {
      totpInputRef.current.focus();
    }
  }, [step]);

  // Attempt a passkey ceremony for the given email.
  // Silently transitions to the password step on any failure.
  async function attemptPasskey(emailValue: string) {
    if (!supportsPasskey) {
      setStep("password");
      return;
    }

    setLoading(true);
    setError(null);

    try {
      const begin = await apiPost<PasskeyLoginBeginResponse>(
        "/auth/mfa/passkey/login/begin",
        { email: emailValue },
      );

      const abort = new AbortController();
      abortRef.current = abort;

      let cred: PublicKeyCredential;
      try {
        cred = (await navigator.credentials.get({
          publicKey: preparePublicKeyOptions(begin.options.publicKey),
          signal: abort.signal,
        })) as PublicKeyCredential;
      } catch {
        setStep("password");
        setLoading(false);
        return;
      }

      if (!cred) {
        setStep("password");
        setLoading(false);
        return;
      }

      const browserId = getCookie("rav_browser");
      const serialized = serializeCredential(cred);

      const response = await apiPost<LoginSuccess>(
        "/auth/mfa/passkey/login/complete",
        {
          nonce: begin.nonce,
          credential: serialized as unknown as Record<string, unknown>,
          remember,
          ...(browserId ? { browser_id: browserId } : {}),
        },
      );

      const accountsData = await fetchAccounts();
      setAccounts(accountsData.accounts);
      setActiveAccount(response.account.id);
      queryClient.clear();
      router.push("/mail");
    } catch {
      setStep("password");
      setLoading(false);
    } finally {
      abortRef.current = null;
    }
  }

  async function handleEmailContinue(e: FormEvent) {
    e.preventDefault();
    // Capture autofilled password into state so the password step is pre-filled
    // if the passkey attempt fails or is cancelled. Always try passkey first.
    const autofilled = hiddenPasswordRef.current?.value ?? "";
    if (autofilled) setPassword(autofilled);
    await attemptPasskey(email);
  }

  async function handlePasswordSubmit(e: FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError(null);

    const browserId = getCookie("rav_browser");
    const payload: Record<string, unknown> = { email, password, remember };
    if (browserId) payload.browser_id = browserId;
    if (totpCode.trim()) payload.totp_code = totpCode.replace(/\s/g, "");

    try {
      const response = await apiPost<LoginResponse>("/auth/login", payload);

      if ("mfa_required" in response && response.mfa_required) {
        setStep("totp");
        setLoading(false);
        return;
      }

      const success = response as LoginSuccess;
      const accountsData = await fetchAccounts();
      setAccounts(accountsData.accounts);
      setActiveAccount(success.account.id);
      queryClient.clear();
      router.push("/mail");
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "An unexpected error occurred",
      );
      setLoading(false);
    }
  }

  if (checking) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <div className="size-8 animate-spin rounded-full border-4 border-muted border-t-primary" />
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4">
      <div className="w-full max-w-sm">
        {/* Branding */}
        <div className="mb-8 text-center">
          <h1 className="text-3xl font-bold tracking-tight text-foreground">
            Rav
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Sign in to your account
          </p>
        </div>

        <Card className="rounded-lg">
          <CardHeader>
            <CardTitle className="text-lg">
              {step === "totp" ? "Two-factor authentication" : "Welcome back"}
            </CardTitle>
            <CardDescription>
              {step === "email" && "Enter your email to continue"}
              {step === "totp" && "Enter the 6-digit code from your authenticator app."}
              {step === "password" && (
                <>
                  Signing in as{" "}
                  <span className="font-medium text-foreground">{email}</span>
                  {" - "}
                  <button
                    type="button"
                    onClick={() => { setStep("email"); setPassword(""); setError(null); }}
                    className="text-primary hover:underline"
                  >
                    Change
                  </button>
                </>
              )}
            </CardDescription>
          </CardHeader>

          <CardContent>
            {error && (
              <div className="mb-4 rounded-md bg-destructive/10 px-4 py-3 text-sm text-destructive wrap-anywhere">
                {error}
              </div>
            )}

            {/* ---- Email step ---- */}
            {step === "email" && (
              <form onSubmit={handleEmailContinue} className="flex flex-col gap-4">
                <div className="flex flex-col gap-2">
                  <Label htmlFor="email">Email</Label>
                  <Input
                    id="email"
                    type="email"
                    placeholder="you@example.com"
                    autoComplete="email"
                    required
                    disabled={loading}
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                  />
                </div>

                {/* Zero-height container keeps the password field in the DOM with real dimensions
                    so password managers detect the email+password pair and offer autofill. */}
                <div className="h-0 overflow-hidden" aria-hidden="true">
                  <input
                    ref={hiddenPasswordRef}
                    type="password"
                    autoComplete="current-password"
                    tabIndex={-1}
                  />
                </div>

                <Button
                  type="submit"
                  className="w-full"
                  size="lg"
                  disabled={loading}
                >
                  {loading ? "Checking..." : "Continue"}
                </Button>

              </form>
            )}

            {/* ---- Password step ---- */}
            {step === "password" && (
              <form
                onSubmit={handlePasswordSubmit}
                className="flex flex-col gap-4"
              >
                <div className="flex flex-col gap-2">
                  <Label htmlFor="password">Password</Label>
                  <Input
                    id="password"
                    type="password"
                    placeholder="Enter your password"
                    autoComplete="current-password"
                    required
                    disabled={loading}
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                  />
                </div>

                <label
                  htmlFor="remember"
                  className="flex items-center gap-2 cursor-pointer select-none"
                >
                  <input
                    id="remember"
                    type="checkbox"
                    checked={remember}
                    onChange={(e) => setRemember(e.target.checked)}
                    disabled={loading}
                    className="size-4 rounded border-muted-foreground/40 accent-primary"
                  />
                  <span className="text-sm text-muted-foreground">
                    Keep me logged in
                  </span>
                </label>

                <Button
                  type="submit"
                  className="w-full"
                  size="lg"
                  disabled={loading}
                >
                  {loading ? "Signing in..." : "Sign in"}
                </Button>

              </form>
            )}

            {/* ---- TOTP step ---- */}
            {step === "totp" && (
              <form
                onSubmit={handlePasswordSubmit}
                className="flex flex-col gap-4"
              >
                <div className="flex flex-col gap-2">
                  <Label htmlFor="totp_code">Verification code</Label>
                  <Input
                    ref={totpInputRef}
                    id="totp_code"
                    type="text"
                    inputMode="numeric"
                    placeholder="000 000"
                    maxLength={7}
                    autoComplete="one-time-code"
                    disabled={loading}
                    value={totpCode}
                    onChange={(e) =>
                      setTotpCode(e.target.value.replace(/[^0-9 ]/g, ""))
                    }
                    className="font-mono tracking-widest text-center text-lg"
                  />
                </div>

                <Button
                  type="submit"
                  className="w-full"
                  size="lg"
                  disabled={
                    loading || totpCode.replace(/\s/g, "").length !== 6
                  }
                >
                  {loading ? "Verifying..." : "Verify"}
                </Button>

                <button
                  type="button"
                  onClick={() => {
                    setStep("email");
                    setPassword("");
                    setTotpCode("");
                    setError(null);
                  }}
                  className="text-sm text-muted-foreground hover:text-foreground text-center transition-colors"
                >
                  Use a different account
                </button>
              </form>
            )}
          </CardContent>
        </Card>

        <p className="mt-6 text-center text-xs text-muted-foreground">
          Secure, private, and fast email.
        </p>
      </div>
    </div>
  );
}
