"use client";

import { useState, useEffect, useLayoutEffect, useRef, useMemo } from "react";
import { AnimatePresence } from "framer-motion";
import {
  ChevronDown,
  ChevronUp,
  Code,
  Type,
  FileCode,
  ShieldAlert,
  Sun,
  Moon,
  Monitor,
  Mail,
  FileText,
  FileImage,
  FileVideo,
  FileAudio,
  FileArchive,
  File,
  Lock,
  ShieldCheck,
  ShieldOff,
  Loader2,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { useUiStore } from "@/stores/useUiStore";
import { useAuthStore } from "@/stores/useAuthStore";
import { useIsMobile } from "@/hooks/useIsMobile";
import { useMessage, useUpdateFlags } from "@/hooks/useMessages";
import { createFadeSlideVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { EmailRenderer, hasRemoteResources } from "./EmailRenderer";
import { ThreadView } from "./ThreadView";
import { Button } from "@/components/ui/button";
import type { MessageDetail } from "@/types/message";
import { usePgpKeys } from "@/hooks/usePgp";
import { decryptMessage, verifySignature } from "@/lib/pgp/client";
import {
  AddressChip,
  AddressList,
  AttachmentPreviewer,
  HeaderSkeleton,
  BodySkeleton,
  formatFileSize,
  humanizeDate,
} from "./ReadingPane/index";

type HeaderMode = "summary" | "details";

function attachmentIcon(contentType: string): LucideIcon {
  if (contentType.startsWith("image/")) return FileImage;
  if (contentType.startsWith("video/")) return FileVideo;
  if (contentType.startsWith("audio/")) return FileAudio;
  if (contentType === "application/pdf") return FileText;
  if (contentType.startsWith("text/")) return FileText;
  if (
    contentType === "application/zip" ||
    contentType === "application/x-zip-compressed" ||
    contentType === "application/x-tar" ||
    contentType === "application/x-gzip" ||
    contentType === "application/x-7z-compressed" ||
    contentType === "application/x-rar-compressed"
  ) return FileArchive;
  return File;
}

type PgpBodyViewProps = {
  message: MessageDetail;
  emailTheme: "light" | "dark" | "auto";
  blockRemoteResources: boolean;
};

function PgpBodyView({ message, emailTheme, blockRemoteResources }: PgpBodyViewProps) {
  const [pgpDecrypted, setPgpDecrypted] = useState<{ text: string; html: string | null; verified: import("@/types/pgp").DecryptResult["verified"] } | null>(null);
  const [pgpDecryptPassphrase, setPgpDecryptPassphrase] = useState("");
  const [pgpDecrypting, setPgpDecrypting] = useState(false);
  const [pgpVerifyStatus, setPgpVerifyStatus] = useState<"idle" | "valid" | "invalid" | "no_key">("idle");
  const { data: pgpKeys } = usePgpKeys();

  useEffect(() => {
    if (!message.pgp_status || message.pgp_status.kind !== "signed") return;
    const { signature, signed_content } = message.pgp_status;
    if (!signature || !signed_content) return;

    // Look up the sender's public key via WKD using the From address.
    // We verify against the sender's key, not the recipient's.
    const senderEmail = message.from_address;

    void (async () => {
      if (!senderEmail) {
        setPgpVerifyStatus("no_key");
        return;
      }
      try {
        const { apiGet } = await import("@/lib/api");
        const { found, public_key } = await apiGet<{ found: boolean; public_key: string | null }>(
          `/pgp/wkd?email=${encodeURIComponent(senderEmail)}`
        );
        if (!found || !public_key) {
          setPgpVerifyStatus("no_key");
          return;
        }
        const { verified } = await verifySignature({
          publicKeyArmored: public_key,
          contentB64: signed_content,
          signature,
        });
        setPgpVerifyStatus(verified);
      } catch {
        setPgpVerifyStatus("no_key");
      }
    })();
  }, [message.pgp_status, message.from_address]);

  async function handleDecrypt() {
    if (!message.pgp_status?.ciphertext || !pgpKeys?.length) return;
    const key = pgpKeys[0];
    let keyRecord: import("@/types/pgp").PgpKeyRecord;
    try {
      keyRecord = await (await import("@/lib/api")).apiGet<import("@/types/pgp").PgpKeyRecord>(`/pgp/keys/${key.id}`);
    } catch {
      return;
    }
    // For encrypted+signed messages, look up the sender's public key so the
    // worker can verify the embedded signature against the correct key.
    let senderPublicKeyArmored: string | undefined;
    if (message.from_address) {
      try {
        const { apiGet } = await import("@/lib/api");
        const wkd = await apiGet<{ found: boolean; public_key: string | null }>(
          `/pgp/wkd?email=${encodeURIComponent(message.from_address)}`
        );
        if (wkd.found && wkd.public_key) {
          senderPublicKeyArmored = wkd.public_key;
        }
      } catch {
        // WKD lookup failure is non-fatal; decrypt will surface 'no_key' verified status.
      }
    }
    setPgpDecrypting(true);
    try {
      const result = await decryptMessage({
        privateKeyArmored: keyRecord.private_key_enc,
        passphrase: pgpDecryptPassphrase,
        ciphertext: message.pgp_status.ciphertext,
        senderPublicKeyArmored,
      });
      setPgpDecrypted({ text: result.text, html: result.html, verified: result.verified });
      setPgpDecryptPassphrase("");
    } catch (e) {
      console.error("Decrypt failed:", e);
    } finally {
      setPgpDecrypting(false);
    }
  }

  return (
    <>
      {message.pgp_status?.kind === "encrypted" && !pgpDecrypted && (
        <div className="m-4 rounded-lg border border-blue-200 bg-blue-50 dark:border-blue-800 dark:bg-blue-950 p-4">
          <div className="flex items-center gap-2 mb-2">
            <Lock className="size-4 text-blue-600 dark:text-blue-400" />
            <span className="text-sm font-medium text-blue-800 dark:text-blue-200">
              This message is PGP encrypted
            </span>
          </div>
          {pgpKeys && pgpKeys.length > 0 ? (
            <div className="flex items-center gap-2 mt-3">
              <input
                type="password"
                placeholder="Key passphrase"
                value={pgpDecryptPassphrase}
                onChange={(e) => setPgpDecryptPassphrase(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleDecrypt(); }}
                className="rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              />
              <button
                type="button"
                onClick={handleDecrypt}
                disabled={!pgpDecryptPassphrase || pgpDecrypting}
                className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
              >
                {pgpDecrypting && <Loader2 className="size-3.5 animate-spin" />}
                Decrypt
              </button>
            </div>
          ) : (
            <p className="text-xs text-blue-700 dark:text-blue-300 mt-1">
              No PGP keys configured. Add a key in Settings to decrypt messages.
            </p>
          )}
        </div>
      )}

      {message.pgp_status?.kind === "signed" && (
        <div className={`mx-4 mt-4 rounded-lg border p-3 flex items-center gap-2 ${
          pgpVerifyStatus === "valid"
            ? "border-green-200 bg-green-50 dark:border-green-800 dark:bg-green-950"
            : pgpVerifyStatus === "invalid"
            ? "border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-950"
            : "border-border bg-muted"
        }`}>
          {pgpVerifyStatus === "valid" ? (
            <ShieldCheck className="size-4 text-green-600 dark:text-green-400 shrink-0" />
          ) : pgpVerifyStatus === "invalid" ? (
            <ShieldOff className="size-4 text-red-600 dark:text-red-400 shrink-0" />
          ) : (
            <ShieldAlert className="size-4 text-muted-foreground shrink-0" />
          )}
          <span className="text-xs font-medium">
            {pgpVerifyStatus === "valid"
              ? `Signed by ${message.from_address ?? "unknown"}`
              : pgpVerifyStatus === "invalid"
              ? "Invalid signature"
              : pgpVerifyStatus === "no_key"
              ? "No key to verify signature"
              : "PGP signed - verifying..."}
          </span>
        </div>
      )}

      {pgpDecrypted?.verified !== "unsigned" && pgpDecrypted && (
        <div className={`mx-4 mt-4 rounded-lg border p-3 flex items-center gap-2 ${
          pgpDecrypted.verified === "valid"
            ? "border-green-200 bg-green-50 dark:border-green-800 dark:bg-green-950"
            : pgpDecrypted.verified === "invalid"
            ? "border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-950"
            : "border-border bg-muted"
        }`}>
          {pgpDecrypted.verified === "valid" ? (
            <ShieldCheck className="size-4 text-green-600 dark:text-green-400 shrink-0" />
          ) : pgpDecrypted.verified === "invalid" ? (
            <ShieldOff className="size-4 text-red-600 dark:text-red-400 shrink-0" />
          ) : (
            <ShieldAlert className="size-4 text-muted-foreground shrink-0" />
          )}
          <span className="text-xs font-medium">
            {pgpDecrypted.verified === "valid"
              ? `Signed by ${message.from_address ?? "unknown"}`
              : pgpDecrypted.verified === "invalid"
              ? "Invalid signature"
              : "No key to verify signature"}
          </span>
        </div>
      )}

      {pgpDecrypted ? (
        <EmailRenderer
          html={pgpDecrypted.html}
          text={pgpDecrypted.text}
          blockRemoteResources={true}
          theme={emailTheme}
          emailTheme={message.email_theme}
        />
      ) : message.pgp_status?.kind === "encrypted" ? null : (
        <EmailRenderer
          html={message.html}
          text={message.text}
          blockRemoteResources={blockRemoteResources}
          theme={emailTheme}
          emailTheme={message.email_theme}
        />
      )}
    </>
  );
}

export function ReadingPane() {
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedMessageUid = useUiStore((s) => s.selectedMessageUid);
  const mobilePanelView = useUiStore((s) => s.mobilePanelView);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const isMobile = useIsMobile();
  const activeAccountId = useAuthStore((s) => s.activeAccountId);
  const [headerMode, setHeaderMode] = useState<HeaderMode>("details");
  const bodyMode = useUiStore((s) => s.readingBodyMode);
  const showHeaders = useUiStore((s) => s.readingShowHeaders);
  const toggleBodyMode = useUiStore((s) => s.toggleReadingBodyMode);
  const toggleShowHeaders = useUiStore((s) => s.toggleReadingShowHeaders);
  const [emailThemeState, setEmailThemeState] = useState<{
    uid: number | null;
    theme: "auto" | "light" | "dark";
  }>({
    uid: null,
    theme: "auto",
  });
  const [allowedRemoteUids, setAllowedRemoteUids] = useState<Set<string>>(
    new Set()
  );
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const queryClient = useQueryClient();
  const selectMessage = useUiStore((s) => s.selectMessage);

  const { data, isLoading, isError, error, refetch } = useMessage(
    activeFolder,
    selectedMessageUid ?? 0
  );

  const updateFlags = useUpdateFlags();
  // Refs so the mark-as-read timer callback always sees fresh values without
  // making them deps that restart the timer on flag changes (e.g. starring).
  const updateFlagsMutateRef = useRef(updateFlags.mutate);
  const activeFolderRef = useRef(activeFolder);
  const dataRef = useRef(data);
  useLayoutEffect(() => {
    updateFlagsMutateRef.current = updateFlags.mutate;
    activeFolderRef.current = activeFolder;
    dataRef.current = data;
  });

  const paneVariants = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "x"), [effectiveAnimationMode]);
  const emailTheme = emailThemeState.uid === data?.uid ? emailThemeState.theme : "auto";

  // When a message is not found on the server (stale cache), deselect it and
  // refresh the message list and search results so the ghost entry disappears.
  const handledStaleRef = useRef<number | null>(null);
  useEffect(() => {
    if (!isError || !error?.message?.includes("not found")) return;
    if (handledStaleRef.current === selectedMessageUid) return;
    handledStaleRef.current = selectedMessageUid;
    selectMessage(null);
    queryClient.invalidateQueries({ queryKey: ["messages", activeFolder] });
    queryClient.invalidateQueries({ queryKey: ["folders"] });
    queryClient.invalidateQueries({ queryKey: ["search"] });
  }, [isError, error, selectedMessageUid, activeFolder, selectMessage, queryClient]);

  // Auto-switch to plain text mode for plaintext-only emails
  useEffect(() => {
    const { setReadingBodyMode } = useUiStore.getState();
    const current = dataRef.current;
    if (current && !current.html && current.text) {
      setReadingBodyMode("plain");
    } else {
      setReadingBodyMode("html");
    }
  }, [data?.uid, data?.html, data?.text]);

  // Auto-mark unread messages as read after a 1.5s dwell timer.
  // On mobile, only start the timer when the reading pane is visible.
  // All mutable values (data, mutate fn, folder) are accessed via refs so that
  // flag changes (e.g. starring) do not restart the timer.
  useEffect(() => {
    if (isMobile && mobilePanelView !== "reading") return;
    const current = dataRef.current;
    if (!current || current.flags.includes("\\Seen")) return;

    const uid = current.uid;
    const timer = setTimeout(() => {
      updateFlagsMutateRef.current({
        folder: activeFolderRef.current,
        uid,
        flags: ["\\Seen"],
        add: true,
      });
    }, 1500);
    return () => clearTimeout(timer);
  }, [data?.uid, data?.folder, mobilePanelView, isMobile]);

  // No message selected
  if (selectedMessageUid === null) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
        <Mail className="size-10 text-muted-foreground/40" strokeWidth={1.25} />
        <p className="text-sm font-medium text-muted-foreground">Select a message from the list</p>
      </div>
    );
  }

  // Loading
  if (isLoading) {
    return (
      <div className="flex h-full flex-col overflow-y-auto w-full">
        <HeaderSkeleton />
        <BodySkeleton />
      </div>
    );
  }

  // Error
  if (isError || !data) {
    const errMsg = error?.message ?? "Unknown error";
    console.error(`[ReadingPane] Failed to load message uid=${selectedMessageUid} folder=${activeFolder}:`, errMsg);
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-4 py-8 text-center w-full">
        <p className="text-sm text-muted-foreground">Failed to load message</p>
        <p className="max-w-md text-xs text-muted-foreground/60">{errMsg}</p>
        <Button variant="outline" size="sm" onClick={() => refetch()}>
          Retry
        </Button>
      </div>
    );
  }

  const attachmentBaseUrl = `${process.env.NEXT_PUBLIC_BASE_PATH || ""}/api/messages/${encodeURIComponent(data.folder)}/${data.uid}/attachments`;
  const messageKey = `${data.folder}:${data.uid}`;
  const remoteAllowed = allowedRemoteUids.has(messageKey);
  const showRemoteBanner = !remoteAllowed && hasRemoteResources(data.html);

  const paneContent = (
    <div
      className="flex h-full w-full flex-col overflow-hidden"
    >
      {/* Header area */}
      <div className="shrink-0 space-y-1 overflow-x-hidden border-b border-border p-4">
        <h2 className="text-lg font-bold leading-tight line-clamp-2 break-words [overflow-wrap:anywhere]">{data.subject}</h2>

        {headerMode === "summary" ? (
          <div className="text-sm text-foreground">
            From{" "}
            <AddressChip
              address={data.from_address}
              name={data.from_name || null}
            />
            {" "}
            on {humanizeDate(data.date)}
          </div>
        ) : (
          <>
            <div className="text-sm text-foreground">
              <span className="font-medium text-muted-foreground">From: </span>
              <AddressChip
                address={data.from_address}
                name={data.from_name || null}
              />
            </div>

            <div className="text-sm text-foreground">
              <span className="font-medium text-muted-foreground">To: </span>
              <AddressList addresses={data.to_addresses} />
            </div>

            {data.cc_addresses.length > 0 && (
              <div className="text-sm text-foreground">
                <span className="font-medium text-muted-foreground">Cc: </span>
                <AddressList addresses={data.cc_addresses} />
              </div>
            )}

            <div className="text-sm text-muted-foreground">
              {humanizeDate(data.date)}
            </div>
          </>
        )}

        {/* View toggle buttons */}
        <div className="flex gap-1 pt-1">
          {/* Details / Summary toggle */}
          <button
            type="button"
            onClick={() =>
              setHeaderMode(headerMode === "details" ? "summary" : "details")
            }
            className="inline-flex items-center gap-1 rounded px-2 py-0.5 text-xs text-muted-foreground transition-colors hover:bg-accent active:bg-accent/70 hover:text-foreground"
          >
            {headerMode === "details" ? (
              <ChevronUp className="size-3" />
            ) : (
              <ChevronDown className="size-3" />
            )}
            {headerMode === "details" ? "Collapse" : "Expand"}
          </button>

          {/* Plain text / HTML toggle - hidden on mobile (accessible via ... menu) */}
          <button
            type="button"
            onClick={toggleBodyMode}
            className={`md:inline-flex hidden items-center gap-1 rounded px-2 py-0.5 text-xs transition-colors ${
              bodyMode === "plain"
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
            }`}
          >
            {bodyMode === "html" ? (
              <Type className="size-3" />
            ) : (
              <Code className="size-3" />
            )}
            {bodyMode === "html" ? "Plain text" : "HTML"}
          </button>

          {/* Headers toggle - hidden on mobile (accessible via ... menu) */}
          <button
            type="button"
            onClick={toggleShowHeaders}
            className={`md:inline-flex hidden items-center gap-1 rounded px-2 py-0.5 text-xs transition-colors ${
              showHeaders
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
            }`}
          >
            <FileCode className="size-3" />
            Headers
          </button>

          {/* Email Theme toggle */}
          <button
            type="button"
            onClick={() => {
              setEmailThemeState((prev) => {
                const baseUid = data.uid;
                const current = prev.uid === baseUid ? prev.theme : "auto";
                const next = current === "auto" ? "light" : current === "light" ? "dark" : "auto";

                return {
                  uid: baseUid,
                  theme: next,
                };
              });
            }}
            className={`inline-flex items-center gap-1 rounded px-2 py-0.5 text-xs transition-colors ${
              emailTheme !== "auto"
                ? "bg-primary text-primary-foreground"
                : "text-muted-foreground hover:bg-accent active:bg-accent/70 hover:text-foreground"
            }`}
          >
            {emailTheme === "auto" && <Monitor className="size-3" />}
            {emailTheme === "light" && <Sun className="size-3" />}
            {emailTheme === "dark" && <Moon className="size-3" />}
            {emailTheme === "auto" ? "Auto theme" : emailTheme === "light" ? "Light mode" : "Dark mode"}
          </button>
        </div>
      </div>

      {/* Attachment bar */}
      {data.attachments.length > 0 && (
        <div className="flex shrink-0 gap-2 overflow-x-auto border-b border-border px-4 py-2">
          {data.attachments.map((att, i) => {
            const Icon = attachmentIcon(att.content_type ?? "");
            return (
              <button
                key={att.id}
                type="button"
                onClick={() => setPreviewIndex(i)}
                className="inline-flex shrink-0 items-center gap-1.5 rounded-md border border-border bg-muted/50 px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-accent active:bg-accent/70"
              >
                <Icon className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="max-w-[200px] truncate">
                  {att.filename ?? "Attachment"}
                </span>
                <span className="text-muted-foreground">
                  ({formatFileSize(att.size)})
                </span>
              </button>
            );
          })}
        </div>
      )}

      {/* Attachment previewer */}
      {previewIndex !== null && (
        <AttachmentPreviewer
          attachments={data.attachments}
          baseUrl={attachmentBaseUrl}
          accountId={activeAccountId}
          initialIndex={previewIndex}
          onClose={() => setPreviewIndex(null)}
        />
      )}

      {/* Thread view — only shown when there are multiple messages in the thread */}
      {data.thread && data.thread.length > 1 && (() => {
        const visibleThread = data.thread.filter(
          (m) => m.folder === activeFolder || m.uid === data.uid,
        );
        return visibleThread.length > 1 ? (
          <ThreadView thread={visibleThread} currentUid={data.uid} />
        ) : null;
      })()}

      {/* Remote resources banner */}
      {showRemoteBanner && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border bg-muted/50 px-4 py-2">
          <ShieldAlert className="size-4 shrink-0 text-muted-foreground" />
          <span className="flex-1 text-xs text-muted-foreground">
            To protect your privacy, remote resources have been blocked.
          </span>
          <Button
            variant="outline"
            size="sm"
            className="h-6 text-xs"
            onClick={() =>
              setAllowedRemoteUids((prev) => new Set(prev).add(messageKey))
            }
          >
            Allow
          </Button>
        </div>
      )}

      {/* Body area — fills remaining space */}
      <div className="min-h-0 flex-1" key={data.uid}>
        {showHeaders ? (
          <pre className="h-full overflow-auto whitespace-pre-wrap break-words p-4 text-xs leading-relaxed text-foreground">
            {data.raw_headers || "No headers available"}
          </pre>
        ) : bodyMode === "plain" ? (
          <pre className="h-full overflow-auto whitespace-pre-wrap break-words p-4 text-sm leading-relaxed text-foreground">
            {data.text || "No plain text available"}
          </pre>
        ) : (
          <PgpBodyView
            key={`${data.folder}-${data.uid}`}
            message={data}
            emailTheme={emailTheme}
            blockRemoteResources={!remoteAllowed}
          />
        )}
      </div>
    </div>
  );

  return (
    <AnimatePresence mode="wait" initial={false}>
      <AnimatedDiv
        key={`reading-pane-${data.uid}`}
        data-testid="reading-pane-message-transition"
        variants={paneVariants}
        initial={paneVariants.initial}
        animate={paneVariants.animate}
        exit={paneVariants.exit}
        className="h-full min-w-0 w-full"
      >
        {paneContent}
      </AnimatedDiv>
    </AnimatePresence>
  );
}
