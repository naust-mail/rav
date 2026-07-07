"use client";

import { useState, useRef, useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence } from "framer-motion";
import {
  ShieldCheck,
  ShieldOff,
  Copy,
  Loader2,
  KeyRound,
  Fingerprint,
  Plus,
  Trash2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { useQueryClient } from "@tanstack/react-query";
import QRCode from "react-qr-code";
import {
  useMfaStatus,
  useTotpSetup,
  useTotpConfirm,
  useTotpDelete,
  usePasskeyList,
  usePasskeyDelete,
  usePasskeySetOnly,
} from "@/hooks/useMfa";
import { apiPost } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { useUiStore } from "@/stores/useUiStore";
import type {
  TotpSetupResponse,
  PasskeyRegisterBeginResponse,
  PasskeyCreationCredential,
  PasskeyRegisterCompleteResponse,
} from "@/types/mfa";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// WebAuthn helpers for registration ceremony
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

function prepareCreationOptions(
  opts: PasskeyRegisterBeginResponse["options"]["publicKey"],
): PublicKeyCredentialCreationOptions {
  return {
    challenge: base64urlToBuffer(opts.challenge),
    rp: opts.rp,
    user: {
      id: base64urlToBuffer(opts.user.id),
      name: opts.user.name,
      displayName: opts.user.displayName,
    },
    pubKeyCredParams: opts.pubKeyCredParams.map((p) => ({
      type: p.type as PublicKeyCredentialType,
      alg: p.alg,
    })),
    timeout: opts.timeout,
    excludeCredentials: (opts.excludeCredentials ?? []).map((c) => ({
      type: c.type as PublicKeyCredentialType,
      id: base64urlToBuffer(c.id),
    })),
    authenticatorSelection: opts.authenticatorSelection as AuthenticatorSelectionCriteria | undefined,
    attestation: opts.attestation as AttestationConveyancePreference | undefined,
    extensions: opts.extensions,
  };
}

function serializeCreationCredential(cred: PublicKeyCredential): PasskeyCreationCredential {
  const attestation = cred.response as AuthenticatorAttestationResponse;
  const ext = cred.getClientExtensionResults() as {
    prf?: { results?: { first?: ArrayBuffer } };
  };
  const transports =
    typeof attestation.getTransports === "function" ? attestation.getTransports() : undefined;
  return {
    id: cred.id,
    rawId: bufferToBase64url(cred.rawId),
    type: cred.type,
    response: {
      clientDataJSON: bufferToBase64url(attestation.clientDataJSON),
      attestationObject: bufferToBase64url(attestation.attestationObject),
      ...(transports ? { transports } : {}),
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

type TotpStep = "idle" | "setup";
type PkStep = "idle" | "naming" | "confirm_delete";

export function SecuritySettings() {
  const queryClient = useQueryClient();
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const overlayMotionProps = useMemo(
    () => createFadeSlideVariants(effectiveAnimationMode, "y"),
    [effectiveAnimationMode],
  );
  const contentMotionProps = useMemo(
    () => createScaleFadeVariants(effectiveAnimationMode),
    [effectiveAnimationMode],
  );

  const { data: status, isLoading } = useMfaStatus();
  const setup = useTotpSetup();
  const confirm = useTotpConfirm();
  const remove = useTotpDelete();

  const [step, setStep] = useState<TotpStep>("idle");
  const [setupData, setSetupData] = useState<TotpSetupResponse | null>(null);
  const [code, setCode] = useState("");
  const [showRemoveDialog, setShowRemoveDialog] = useState(false);
  const [removeCode, setRemoveCode] = useState("");
  const codeInputRef = useRef<HTMLInputElement>(null);
  const removeCodeInputRef = useRef<HTMLInputElement>(null);

  const pkList = usePasskeyList();
  const pkDelete = usePasskeyDelete();
  const pkSetOnly = usePasskeySetOnly();

  const [pkStep, setPkStep] = useState<PkStep>("idle");
  const [newKeyName, setNewKeyName] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [pkError, setPkError] = useState<string | null>(null);
  const [pkLoading, setPkLoading] = useState(false);

  useEffect(() => {
    if (step === "setup" && codeInputRef.current) {
      codeInputRef.current.focus();
    }
  }, [step]);

  useEffect(() => {
    if (!showRemoveDialog) return;
    const t = setTimeout(() => removeCodeInputRef.current?.focus(), 50);
    return () => clearTimeout(t);
  }, [showRemoveDialog]);

  // Close remove dialog on Escape
  useEffect(() => {
    if (!showRemoveDialog) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") setShowRemoveDialog(false);
    }
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [showRemoveDialog]);

  async function handleStartSetup() {
    try {
      const data = await setup.mutateAsync();
      setSetupData(data);
      setCode("");
      setStep("setup");
    } catch {
      toast.error("Failed to generate setup code. Try again.");
    }
  }

  async function handleConfirm() {
    if (!setupData || code.replace(/\s/g, "").length !== 6) return;
    try {
      await confirm.mutateAsync({ secret: setupData.secret, code: code.replace(/\s/g, "") });
      toast.success("Authenticator app enabled");
      setStep("idle");
      setSetupData(null);
      setCode("");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Invalid code. Check your app and try again.");
    }
  }

  async function handleRemove() {
    if (removeCode.length !== 6) return;
    try {
      await remove.mutateAsync(removeCode);
      toast.success("Authenticator app removed");
      setShowRemoveDialog(false);
      setRemoveCode("");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Invalid code. Check your app and try again.");
    }
  }

  function copySecret() {
    if (!setupData) return;
    navigator.clipboard.writeText(setupData.secret).then(() => {
      toast.success("Secret copied");
    });
  }

  async function handleEnrollPasskey() {
    setPkLoading(true);
    setPkError(null);
    try {
      const begin = await apiPost<PasskeyRegisterBeginResponse>("/mfa/passkey/register/begin", {
        name: newKeyName.trim(),
      });

      let cred: PublicKeyCredential;
      try {
        cred = (await navigator.credentials.create({
          publicKey: prepareCreationOptions(begin.options.publicKey),
        })) as PublicKeyCredential;
      } catch {
        setPkError("Passkey enrollment was cancelled or your browser did not complete the request.");
        setPkLoading(false);
        return;
      }

      if (!cred) {
        setPkError("No credential returned. Try again.");
        setPkLoading(false);
        return;
      }

      await apiPost<PasskeyRegisterCompleteResponse>("/mfa/passkey/register/complete", {
        nonce: begin.nonce,
        credential: serializeCreationCredential(cred) as unknown as Record<string, unknown>,
      });

      toast.success(`Passkey "${newKeyName.trim() || "Passkey"}" added`);
      setNewKeyName("");
      setPkStep("idle");
      queryClient.invalidateQueries({ queryKey: ["mfa", "passkeys"] });
      queryClient.invalidateQueries({ queryKey: ["mfa", "status"] });
    } catch (err) {
      setPkError(err instanceof Error ? err.message : "Failed to add passkey.");
    } finally {
      setPkLoading(false);
    }
  }

  async function handleDeletePasskey(id: string) {
    try {
      await pkDelete.mutateAsync(id);
      toast.success("Passkey removed");
      setConfirmDeleteId(null);
      setPkStep("idle");
    } catch {
      toast.error("Failed to remove passkey.");
    }
  }

  async function handleSetPasskeyOnly(enabled: boolean) {
    try {
      await pkSetOnly.mutateAsync(enabled);
      toast.success(enabled ? "Passkey-only mode enabled" : "Passkey-only mode disabled");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to update setting.");
    }
  }

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground py-8">
        <Loader2 className="size-4 animate-spin" />
        Loading security settings...
      </div>
    );
  }

  const totpEnabled = status?.totp_enabled ?? false;
  const passkeyCount = status?.passkey_count ?? 0;
  const passkeys = pkList.data?.passkeys ?? [];

  return (
    <div className="flex flex-col gap-6 max-w-2xl">
      <div>
        <h2 className="text-base font-semibold">Security</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          Manage two-factor authentication for your webmail session.
        </p>
      </div>

      {/* TOTP status card */}
      <div className="rounded-lg border border-border p-4">
        <div className="flex items-start gap-3">
          <div className={cn(
            "mt-0.5 rounded-md p-1.5",
            totpEnabled ? "bg-green-500/10 text-green-600 dark:text-green-400" : "bg-muted text-muted-foreground"
          )}>
            {totpEnabled ? <ShieldCheck className="size-4" /> : <ShieldOff className="size-4" />}
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium">Authenticator app</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {totpEnabled
                ? "Two-factor authentication is active. You will need your app to sign in."
                : "Add a second factor to protect your webmail account. Requires an app like Google Authenticator or Authy."}
            </p>
          </div>
          <span className={cn(
            "shrink-0 text-xs font-medium px-2 py-0.5 rounded-full",
            totpEnabled
              ? "bg-green-500/10 text-green-700 dark:text-green-400"
              : "bg-muted text-muted-foreground"
          )}>
            {totpEnabled ? "Active" : "Off"}
          </span>
        </div>

        {/* Action area */}
        {step === "idle" && (
          <div className="mt-4 flex gap-2">
            {totpEnabled ? (
              <Button
                variant="destructive"
                size="sm"
                onClick={() => { setRemoveCode(""); setShowRemoveDialog(true); }}
              >
                <Trash2 className="size-3.5" />
                Disable
              </Button>
            ) : (
              <button
                type="button"
                onClick={handleStartSetup}
                disabled={setup.isPending}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
              >
                {setup.isPending ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <KeyRound className="size-3.5" />
                )}
                Set up authenticator app
              </button>
            )}
          </div>
        )}

        {/* Setup flow */}
        {step === "setup" && setupData && (
          <div className="mt-4 ml-3 mr-3 mb-3 space-y-6 pt-3 border-t border-border">
            <div className="space-y-3">
              <h3 className="text-sm font-medium">Complete your setup</h3>
              <div className="flex flex-col sm:flex-row gap-6">
                <div className="shrink-0 rounded-md bg-white p-3 self-start">
                  <QRCode value={setupData.url} size={128} level="M" />
                </div>
                <div className="space-y-3 flex-1">
                  <p className="text-xs text-muted-foreground">
                    1. Scan the QR code, or enter the secret manually.
                  </p>
                  <div className="flex items-center gap-2 max-w-md">
                    <code className="flex-1 rounded-md border bg-muted px-3 py-1.5 font-mono text-xs break-all">
                      {setupData.secret}
                    </code>
                    <button onClick={copySecret} className="p-1.5 rounded-md border hover:bg-accent">
                      <Copy className="size-4" />
                    </button>
                  </div>
                  <div className="flex flex-col gap-2">
                    <p className="text-xs font-medium text-muted-foreground">
                      2. Enter the 6-digit code from your app to activate.
                    </p>
                    <div className="flex gap-2">
                      <input
                        ref={codeInputRef}
                        type="text"
                        inputMode="numeric"
                        pattern="[0-9]*"
                        maxLength={6}
                        placeholder="000000"
                        value={code}
                        onChange={(e) => setCode(e.target.value.replace(/[^0-9 ]/g, ""))}
                        onKeyDown={(e) => { if (e.key === "Enter") handleConfirm(); }}
                        className="w-[9ch] box-content rounded-md border border-border bg-background px-2 py-1 text-sm tracking-[0.5ch] font-mono focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
                      />
                      <button
                        type="button"
                        onClick={handleConfirm}
                        disabled={confirm.isPending || code.replace(/\s/g, "").length !== 6}
                        className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
                      >
                        {confirm.isPending && <Loader2 className="size-3.5 animate-spin" />}
                        Activate
                      </button>
                      <button
                        type="button"
                        onClick={() => { setStep("idle"); setSetupData(null); setCode(""); }}
                        className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Passkey card */}
      <div className="rounded-lg border border-border p-4">
        <div className="flex items-start gap-3">
          <div className={cn(
            "mt-0.5 rounded-md p-1.5",
            passkeyCount > 0
              ? "bg-green-500/10 text-green-600 dark:text-green-400"
              : "bg-muted text-muted-foreground"
          )}>
            <Fingerprint className="size-4" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-sm font-medium">Passkeys</div>
            <p className="mt-0.5 text-xs text-muted-foreground">
              {passkeyCount > 0
                ? "Sign in with a biometric or hardware key, without entering your password."
                : "Add a passkey to sign in using Touch ID, Face ID, Windows Hello, or a hardware key."}
            </p>
          </div>
          <span className={cn(
            "shrink-0 text-xs font-medium px-2 py-0.5 rounded-full",
            passkeyCount > 0
              ? "bg-green-500/10 text-green-700 dark:text-green-400"
              : "bg-muted text-muted-foreground"
          )}>
            {passkeyCount > 0 ? `${passkeyCount} active` : "None"}
          </span>
        </div>

        {/* Passkey list */}
        {passkeys.length > 0 && (
          <div className="mt-4 flex flex-col gap-2">
            {passkeys.map((pk) => (
              <div key={pk.id}>
                <div className="flex items-center justify-between rounded-md border border-border px-3 py-2">
                  <div className="flex flex-col gap-0.5">
                    <span className="text-sm font-medium">{pk.name}</span>
                    <span className="text-xs text-muted-foreground">
                      Enrolled {new Date(pk.created_at).toLocaleDateString()}
                    </span>
                  </div>
                  <Button
                    variant="destructive"
                    size="xs"
                    onClick={() => {
                      setConfirmDeleteId(pk.id);
                      setPkStep("confirm_delete");
                      setPkError(null);
                    }}
                    disabled={pkDelete.isPending && confirmDeleteId === pk.id}
                  >
                    <Trash2 />
                    Remove
                  </Button>
                </div>
                {pkStep === "confirm_delete" && confirmDeleteId === pk.id && (
                  <div className="mt-1 rounded-md bg-destructive/10 p-3">
                    <p className="text-sm text-destructive font-medium">Remove &quot;{pk.name}&quot;?</p>
                    <p className="mt-1 text-xs text-destructive/80">
                      You will no longer be able to sign in with this key.
                      {status?.passkey_only && passkeyCount === 1 && (
                        " Passkey-only mode is on - removing your last key may lock you out."
                      )}
                    </p>
                    <div className="mt-3 flex gap-2">
                      <Button
                        variant="destructive"
                        size="sm"
                        onClick={() => handleDeletePasskey(pk.id)}
                        disabled={pkDelete.isPending}
                      >
                        {pkDelete.isPending && <Loader2 className="size-3.5 animate-spin" />}
                        Remove
                      </Button>
                      <button
                        type="button"
                        onClick={() => { setPkStep("idle"); setConfirmDeleteId(null); }}
                        disabled={pkDelete.isPending}
                        className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}

        {/* Error */}
        {pkError && (
          <div className="mt-3 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {pkError}
          </div>
        )}

        {/* Add passkey / naming */}
        {pkStep === "idle" && (
          <div className="mt-4">
            <button
              type="button"
              onClick={() => { setPkStep("naming"); setPkError(null); }}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 transition-colors"
            >
              <Plus className="size-3.5" />
              Add passkey
            </button>
          </div>
        )}

        {pkStep === "naming" && (
          <div className="mt-4 flex gap-2">
            <input
              type="text"
              placeholder="Name (e.g. YubiKey, MacBook)"
              value={newKeyName}
              onChange={(e) => setNewKeyName(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleEnrollPasskey(); }}
              disabled={pkLoading}
              className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
            />
            <button
              type="button"
              onClick={handleEnrollPasskey}
              disabled={pkLoading}
              className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
            >
              {pkLoading && <Loader2 className="size-3.5 animate-spin" />}
              {pkLoading ? "Enrolling..." : "Enroll"}
            </button>
            <button
              type="button"
              onClick={() => { setPkStep("idle"); setNewKeyName(""); setPkError(null); }}
              disabled={pkLoading}
              className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
            >
              Cancel
            </button>
          </div>
        )}

        {/* Passkey-only toggle */}
        {passkeyCount > 0 && pkStep !== "naming" && (
          <div className="mt-4 pt-4 border-t border-border">
            <label className="flex items-start gap-3 cursor-pointer select-none">
              <input
                type="checkbox"
                checked={status?.passkey_only ?? false}
                onChange={(e) => handleSetPasskeyOnly(e.target.checked)}
                disabled={pkSetOnly.isPending}
                className="mt-0.5 size-4 rounded border-muted-foreground/40 accent-primary"
              />
              <div>
                <div className="text-sm font-medium">Passkey-only sign-in</div>
                <p className="mt-0.5 text-xs text-muted-foreground">
                  Disables password and TOTP login. Recovery requires an admin to reset your account.
                </p>
              </div>
            </label>
          </div>
        )}
      </div>

      <p className="text-xs text-muted-foreground">
        Note: Two-factor authentication protects your webmail session only.
        Native mail apps (iOS Mail, Thunderbird, etc.) connect directly via IMAP and are not affected.
      </p>

      {/* TOTP removal dialog */}
      {typeof document !== "undefined" &&
        createPortal(
          <AnimatePresence>
            {showRemoveDialog && (
              <div className="fixed inset-0 z-50 flex items-center justify-center">
                <AnimatedDiv
                  variants={overlayMotionProps}
                  initial={overlayMotionProps.initial}
                  animate={overlayMotionProps.animate}
                  exit={overlayMotionProps.exit}
                  className="absolute inset-0 bg-black/50"
                  onClick={() => setShowRemoveDialog(false)}
                />
                <AnimatedDiv
                  variants={contentMotionProps}
                  initial={contentMotionProps.initial}
                  animate={contentMotionProps.animate}
                  exit={contentMotionProps.exit}
                  className="relative z-10 w-full max-w-sm rounded-lg border border-border bg-background p-6 shadow-xl"
                >
                  <div className="flex items-center justify-between mb-4">
                    <h2 className="text-base font-semibold">Disable authenticator app</h2>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={() => setShowRemoveDialog(false)}
                      className="text-muted-foreground"
                    >
                      <X className="size-4" />
                    </Button>
                  </div>

                  <p className="text-sm text-muted-foreground mb-4">
                    Enter your current 6-digit code to confirm. Your account will only be protected by your password after this.
                  </p>

                  <div className="space-y-4">
                    <div className="space-y-1.5">
                      <Label htmlFor="remove-totp-code">Verification code</Label>
                      <Input
                        ref={removeCodeInputRef}
                        id="remove-totp-code"
                        type="text"
                        inputMode="numeric"
                        pattern="[0-9]*"
                        maxLength={6}
                        placeholder="000000"
                        value={removeCode}
                        onChange={(e) => setRemoveCode(e.target.value.replace(/[^0-9]/g, ""))}
                        onKeyDown={(e) => { if (e.key === "Enter") handleRemove(); }}
                        disabled={remove.isPending}
                        className="font-mono tracking-widest text-center"
                      />
                    </div>

                    <div className="flex gap-2">
                      <Button
                        variant="destructive"
                        className="flex-1"
                        onClick={handleRemove}
                        disabled={remove.isPending || removeCode.length !== 6}
                      >
                        {remove.isPending && <Loader2 className="size-4 animate-spin" />}
                        Disable
                      </Button>
                      <Button
                        variant="outline"
                        onClick={() => setShowRemoveDialog(false)}
                        disabled={remove.isPending}
                      >
                        Cancel
                      </Button>
                    </div>
                  </div>
                </AnimatedDiv>
              </div>
            )}
          </AnimatePresence>,
          document.body,
        )}
    </div>
  );
}
