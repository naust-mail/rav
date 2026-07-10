"use client";

import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { AnimatePresence, motion } from "framer-motion";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import {
  Send,
  X,
  AlertTriangle,
  Paperclip,
  Loader2,
  Save,
  Maximize2,
  Minimize2,
  Upload,
  FileText,
  Code,
  PenLine,
  Lock,
  LockOpen,
  ShieldCheck,
} from "lucide-react";
import { toast } from "sonner";
import {
  useComposeStore,
  replaceSignatureInBody,
  removeSignatureFromBody,
  injectSignature,
} from "@/stores/useComposeStore";
import {
  useSendMessage,
  useSaveDraft,
  useUploadAttachment,
  useDeleteAttachment,
  useDeleteDraft,
} from "@/hooks/useCompose";
import { useIdentities } from "@/hooks/useIdentities";
import { usePgpKeys, useServerCapability } from "@/hooks/usePgp";
import { signContent, encryptMessage } from "@/lib/pgp/client";
import { lookupWkd } from "@/lib/pgp/wkd";
import type { PgpKeyRecord, PgpSendRequest } from "@/types/pgp";
import { useDisplayPreferences } from "@/hooks/useDisplayPreferences";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/useUiStore";
import { useIsMobile } from "@/hooks/useIsMobile";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { getMotionTokens } from "@/lib/motion/config";
import {
  countRecipients,
  formatFileSize,
  stripHtml,
  generateId,
  DiscardAlertDialog,
  AttachmentPreviewDialog,
} from "./ComposeDialog/index";
import { RecipientInput } from "./ComposeDialog/RecipientInput";
import dynamic from "next/dynamic";

const RichTextEditor = dynamic(
  () => import("@/components/mail/RichTextEditor").then((mod) => mod.RichTextEditor),
  { ssr: false }
);

export function ComposeDialog() {
  const {
    isOpen,
    mode,
    to,
    cc,
    bcc,
    subject,
    body,
    quotedHtml,
    quotedText,
    inReplyTo,
    references,
    draftId,
    showCc,
    showBcc,
    attachments,
    fromIdentityId,
    isHtml,
    signatureHtml,
    signatureEnabled,
    closeCompose,
    setField,
    setShowCc,
    setShowBcc,
    setDraftId,
    setFromIdentityId,
    setIsHtml,
    setSignatureHtml,
    setSignatureEnabled,
    addAttachments,
    removeAttachment,
    reset,
    pgpMode,
    setPgpMode,
  } = useComposeStore();

  const sendMutation = useSendMessage();
  const pgpCapable = useServerCapability("pgp");
  const { data: pgpKeys } = usePgpKeys();
  const hasPgpKeys = pgpCapable && (pgpKeys?.length ?? 0) > 0;
  const [pgpPassphrase, setPgpPassphrase] = useState("");
  const [showPgpPassphraseModal, setShowPgpPassphraseModal] = useState(false);
  const [pgpSending, setPgpSending] = useState(false);
  const saveDraftMutation = useSaveDraft();
  const uploadMutation = useUploadAttachment();
  const deleteMutation = useDeleteAttachment();
  const deleteDraftMutation = useDeleteDraft();
  const { data: identities } = useIdentities();
  const { data: displayPrefs } = useDisplayPreferences();
  const [draftSaved, setDraftSaved] = useState(false);
  const [showDiscardAlert, setShowDiscardAlert] = useState(false);
  const [missingKeysPending, setMissingKeysPending] = useState<{
    recipients: string[];
    pubKeys: string[];
    keyRecord: PgpKeyRecord;
    passphrase: string;
  } | null>(null);
  const [quoteExpanded, setQuoteExpanded] = useState(false);
  const isMobile = useIsMobile();
  const [expanded, setExpanded] = useState(false);
  const [previewAttId, setPreviewAttId] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const dragCounterRef = useRef(0);
  const toInputRef = useRef<HTMLInputElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const lastSavedHashRef = useRef<string>("");
  const [lastSavedHash, setLastSavedHash] = useState("");
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const shouldAnimate = effectiveAnimationMode !== "off";
  const motionTokens = useMemo(() => getMotionTokens(effectiveAnimationMode), [effectiveAnimationMode]);
  const overlayMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const contentMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const sendFeedbackMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const serializedSendFeedbackMotionProps = useMemo(() => JSON.stringify(sendFeedbackMotionProps), [sendFeedbackMotionProps]);

  const hasContent = useCallback(() => {
    return !!(
      to.trim() ||
      cc.trim() ||
      bcc.trim() ||
      subject.trim() ||
      stripHtml(body).trim() ||
      attachments.length > 0
    );
  }, [to, cc, bcc, subject, body, attachments]);

  // Auto-select the default identity when dialog opens and no identity is set,
  // and inject signature into body
  useEffect(() => {
    if (isOpen && fromIdentityId === null && identities && identities.length > 0) {
      const defaultIdentity = identities.find((i) => i.is_default) ?? identities[0];
      setFromIdentityId(defaultIdentity.id);
      // Inject signature from the default identity
      if (defaultIdentity.signature_html && signatureEnabled) {
        setSignatureHtml(defaultIdentity.signature_html);
        setField("body", injectSignature(body, defaultIdentity.signature_html));
      }
    }
  }, [isOpen, fromIdentityId, identities, setFromIdentityId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Auto-focus: body editor for replies/forwards (To is pre-filled), To field for new messages
  useEffect(() => {
    if (isOpen) {
      const rafId = requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (mode === "reply" || mode === "forward") {
            const editor = document.querySelector<HTMLElement>(
              "[data-compose-body] .ProseMirror, [data-compose-body] textarea"
            );
            editor?.focus();
          } else {
            toInputRef.current?.focus();
          }
        });
      });
      return () => cancelAnimationFrame(rafId);
    }
  }, [isOpen, mode]);

  // Compute a simple hash of compose fields for dirty tracking
  const computeHash = useCallback(() => {
    return `${to}|${cc}|${bcc}|${subject}|${body}`;
  }, [to, cc, bcc, subject, body]);

  // Save draft function. When force=true, skip the hash guard (used for explicit save/close).
  const saveDraft = useCallback(
    (force = false) => {
      const hash = computeHash();
      // Don't save if nothing changed (unless forced) or compose is empty
      if (!force && hash === lastSavedHashRef.current) return;
      if (
        !to.trim() &&
        !cc.trim() &&
        !bcc.trim() &&
        !subject.trim() &&
        !stripHtml(body).trim()
      )
        return;

      let currentDraftId = draftId;
      if (!currentDraftId) {
        currentDraftId = generateId();
        setDraftId(currentDraftId);
      }

      // Store only the user's typed content - quoted content is reconstructed
      // from the original message (via in_reply_to) when the draft is reopened.
      const draftHtmlBody = isHtml ? body : null;
      const draftTextBody = isHtml ? stripHtml(body) : body;
      saveDraftMutation.mutate(
        {
          uuid: currentDraftId,
          to,
          cc,
          bcc,
          subject,
          textBody: draftTextBody,
          htmlBody: draftHtmlBody,
          inReplyTo: inReplyTo,
          references: references,
        },
        {
          onSuccess: () => {
            lastSavedHashRef.current = hash;
            setLastSavedHash(hash);
            setDraftSaved(true);
            setTimeout(() => setDraftSaved(false), 3000);
          },
        }
      );
    },
    [
      computeHash,
      to,
      cc,
      bcc,
      subject,
      body,
      isHtml,
      draftId,
      setDraftId,
      inReplyTo,
      references,
      saveDraftMutation,
    ]
  );

  // Auto-save every 15s when dialog is open
  useEffect(() => {
    if (!isOpen) return;
    const interval = setInterval(() => {
      saveDraft();
    }, 15000);
    return () => clearInterval(interval);
  }, [isOpen, saveDraft]);

  // Reset saved hash when dialog opens
  useEffect(() => {
    if (isOpen) {
      const h = computeHash();
      lastSavedHashRef.current = h;
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setLastSavedHash(h);
    }
  }, [isOpen]); // eslint-disable-line react-hooks/exhaustive-deps

  const doSend = useCallback((pgpParams?: PgpSendRequest | null) => {
    let plainText: string;
    let sendHtml: string | null;
    if (isHtml) {
      const fullHtml = quotedHtml ? body + quotedHtml : body;
      plainText = stripHtml(body) + (quotedText ? "\n" + quotedText : "");
      // Convert preview URLs back to cid: references for the email MIME body
      sendHtml = fullHtml.replace(
        /\/api\/drafts\/[^/]+\/attachments\/([^/]+)\/content/g,
        (_match, attId) => `cid:${attId}`
      );
    } else {
      plainText = body + (quotedText ? "\n" + quotedText : "");
      sendHtml = null;
    }
    sendMutation.mutate(
      {
        to,
        cc,
        bcc,
        subject,
        body: plainText,
        htmlBody: sendHtml,
        inReplyTo,
        references,
        draftId,
        fromIdentityId,
        pgp: pgpParams ?? undefined,
      },
      {
        onSuccess: () => {
          toast.success("Message sent");
          reset();
        },
        onError: (error) => {
          toast.error(`Failed to send: ${error.message}`);
        },
      }
    );
  }, [
    to,
    cc,
    bcc,
    subject,
    body,
    quotedHtml,
    quotedText,
    isHtml,
    inReplyTo,
    references,
    draftId,
    fromIdentityId,
    sendMutation,
    reset,
  ]);

  // Handle PGP signing/encryption before the actual send.
  const doEncryptAndSend = useCallback(async (
    pubKeys: string[],
    keyRecord: PgpKeyRecord,
    passphrase: string,
  ) => {
    const textContent = stripHtml(body) + (quotedText ? "\n" + quotedText : "");
    const { ciphertext } = await encryptMessage({
      publicKeyArmoreds: pubKeys,
      content: textContent,
      privateKeyArmored: keyRecord.private_key_enc,
      passphrase,
    });
    const pgpParams: PgpSendRequest = { mode: "encrypt", signature: null, ciphertext, micalg: "pgp-sha256" };
    closeCompose();
    const delay = (displayPrefs?.undo_send_delay ?? 5) * 1000;
    if (delay === 0) {
      doSend(pgpParams);
      return;
    }
    const timer = setTimeout(() => {
      toast.dismiss("undo-send");
      doSend(pgpParams);
    }, delay);
    toast("Sending encrypted message...", {
      id: "undo-send",
      duration: delay + 500,
      action: {
        label: "Undo",
        onClick: () => {
          clearTimeout(timer);
          useComposeStore.setState({ isOpen: true });
        },
      },
    });
  }, [body, quotedText, closeCompose, displayPrefs, doSend]);

  const handleMissingKeysConfirm = useCallback(async () => {
    if (!missingKeysPending) return;
    const { pubKeys, keyRecord, passphrase } = missingKeysPending;
    setMissingKeysPending(null);
    setPgpSending(true);
    try {
      await doEncryptAndSend(pubKeys, keyRecord, passphrase);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "PGP operation failed");
    } finally {
      setPgpSending(false);
      setPgpPassphrase("");
    }
  }, [missingKeysPending, doEncryptAndSend]);

  const handlePgpSend = useCallback(async (passphrase: string) => {
    setShowPgpPassphraseModal(false);
    if (!pgpKeys || pgpKeys.length === 0) return;

    setPgpSending(true);
    try {
      const key = pgpKeys[0];
      const keyRecord = await (await import("@/lib/api")).apiGet<PgpKeyRecord>(`/pgp/keys/${key.id}`);
      let pgpParams: PgpSendRequest;

      if (pgpMode === "sign") {
        const textContent = stripHtml(body) + (quotedText ? "\n" + quotedText : "");
        const { signature, micalg } = await signContent({
          privateKeyArmored: keyRecord.private_key_enc,
          passphrase,
          content: textContent,
        });
        pgpParams = { mode: "sign", signature, ciphertext: null, micalg };

        closeCompose();
        const delay = (displayPrefs?.undo_send_delay ?? 5) * 1000;
        if (delay === 0) {
          doSend(pgpParams);
          return;
        }
        const timer = setTimeout(() => {
          toast.dismiss("undo-send");
          doSend(pgpParams);
        }, delay);
        toast("Sending signed message...", {
          id: "undo-send",
          duration: delay + 500,
          action: {
            label: "Undo",
            onClick: () => {
              clearTimeout(timer);
              useComposeStore.setState({ isOpen: true });
            },
          },
        });
      } else {
        // Encrypt: look up recipient public keys via WKD.
        const senderEmail = identities?.find((i) => i.id === fromIdentityId)?.email ?? "";
        const recipients = [...new Set(
          [...to.split(","), ...cc.split(","), ...bcc.split(",")]
            .map((r) => r.trim())
            .filter((r) => r && r !== senderEmail)
        )];

        const results = await Promise.all(
          recipients.map((r) => lookupWkd(r).catch(() => null))
        );

        const pubKeys: string[] = [key.public_key];
        const missingRecipients: string[] = [];
        for (let i = 0; i < recipients.length; i++) {
          if (results[i]) {
            pubKeys.push(results[i]!);
          } else {
            missingRecipients.push(recipients[i]);
          }
        }

        if (missingRecipients.length > 0) {
          // Pause and show the missing-keys dialog; resume via handleMissingKeysConfirm.
          setMissingKeysPending({ recipients: missingRecipients, pubKeys, keyRecord, passphrase });
          return;
        }

        await doEncryptAndSend(pubKeys, keyRecord, passphrase);
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "PGP operation failed");
    } finally {
      setPgpSending(false);
      setPgpPassphrase("");
    }
  }, [pgpMode, pgpKeys, body, quotedText, to, cc, bcc, closeCompose, displayPrefs, doSend, doEncryptAndSend, fromIdentityId, identities]);

  const handleSend = useCallback(() => {
    if (!to.trim() && !cc.trim() && !bcc.trim()) return;

    // If PGP mode is active and we have keys, prompt for passphrase first.
    if (pgpMode !== "off" && hasPgpKeys) {
      setShowPgpPassphraseModal(true);
      return;
    }

    closeCompose();

    const delay = (displayPrefs?.undo_send_delay ?? 5) * 1000;

    if (delay === 0) {
      doSend();
      return;
    }

    const timer = setTimeout(() => {
      toast.dismiss("undo-send");
      doSend();
    }, delay);

    toast("Sending message...", {
      id: "undo-send",
      duration: delay + 500,
      action: {
        label: "Undo",
        onClick: () => {
          clearTimeout(timer);
          useComposeStore.setState({ isOpen: true });
        },
      },
    });
  }, [to, cc, bcc, closeCompose, doSend, displayPrefs?.undo_send_delay, pgpMode, hasPgpKeys]);

  const handleDiscard = useCallback(() => {
    if (hasContent()) {
      setShowDiscardAlert(true);
    } else {
      reset();
    }
  }, [hasContent, reset]);

  const confirmDiscard = useCallback(() => {
    setShowDiscardAlert(false);
    if (draftId) {
      deleteDraftMutation.mutate(draftId);
    }
    reset();
  }, [draftId, deleteDraftMutation, reset]);

  // Save draft and close the dialog without discarding.
  const handleSaveAndClose = useCallback(() => {
    // Only save if the user actually changed something - pre-filled reply fields
    // (to/subject) count as content but not as a reason to save a draft.
    if (hasContent() && computeHash() !== lastSavedHashRef.current) {
      saveDraft(true);
    }
    closeCompose();
  }, [hasContent, saveDraft, closeCompose, computeHash]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        handleSend();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        saveDraft(true);
      }
    },
    [handleSend, saveDraft]
  );

  const handleAttachFiles = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileSelected = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files;
      if (!files || files.length === 0) return;

      // Ensure we have a draft ID for uploads
      let currentDraftId = draftId;
      if (!currentDraftId) {
        currentDraftId = generateId();
        setDraftId(currentDraftId);
      }

      uploadMutation.mutate(
        { draftId: currentDraftId, files: Array.from(files) },
        {
          onSuccess: (data) => {
            addAttachments(
              data.attachments.map((a) => ({
                id: a.id,
                filename: a.filename,
                contentType: a.content_type,
                size: a.size,
              }))
            );
          },
          onError: (error) => {
            toast.error(`Upload failed: ${error.message}`);
          },
        }
      );

      // Reset the input so the same file can be re-selected
      e.target.value = "";
    },
    [draftId, setDraftId, uploadMutation, addAttachments]
  );

  const handleImageUpload = useCallback(
    async (file: File): Promise<string | null> => {
      // Ensure we have a draft ID for the upload
      let currentDraftId = draftId;
      if (!currentDraftId) {
        currentDraftId = generateId();
        setDraftId(currentDraftId);
      }

      return new Promise((resolve) => {
        uploadMutation.mutate(
          { draftId: currentDraftId!, files: [file] },
          {
            onSuccess: (data) => {
              if (data.attachments.length > 0) {
                const att = data.attachments[0];
                addAttachments([
                  {
                    id: att.id,
                    filename: att.filename,
                    contentType: att.content_type,
                    size: att.size,
                  },
                ]);
                // Return a preview URL that the browser can render.
                // The send flow converts these back to cid: references.
                resolve(
                  `${process.env.NEXT_PUBLIC_BASE_PATH || ""}/api/drafts/${currentDraftId}/attachments/${att.id}/content`
                );
              } else {
                resolve(null);
              }
            },
            onError: (error) => {
              toast.error(`Image upload failed: ${error.message}`);
              resolve(null);
            },
          }
        );
      });
    },
    [draftId, setDraftId, uploadMutation, addAttachments]
  );

  const handleRemoveAttachment = useCallback(
    (attachmentId: string) => {
      if (!draftId) return;
      deleteMutation.mutate(
        { draftId, attachmentId },
        {
          onSuccess: () => {
            removeAttachment(attachmentId);
          },
          onError: (error) => {
            toast.error(`Delete failed: ${error.message}`);
          },
        }
      );
    },
    [draftId, deleteMutation, removeAttachment]
  );

  const handleDragEnter = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current += 1;
    if (e.dataTransfer.types.includes("Files")) {
      setIsDragging(true);
    }
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragCounterRef.current -= 1;
    if (dragCounterRef.current === 0) {
      setIsDragging(false);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragging(false);
      dragCounterRef.current = 0;

      const files = Array.from(e.dataTransfer.files);
      if (files.length === 0) return;

      let currentDraftId = draftId;
      if (!currentDraftId) {
        currentDraftId = generateId();
        setDraftId(currentDraftId);
      }

      uploadMutation.mutate(
        { draftId: currentDraftId, files },
        {
          onSuccess: (data) => {
            addAttachments(
              data.attachments.map((a) => ({
                id: a.id,
                filename: a.filename,
                contentType: a.content_type,
                size: a.size,
              }))
            );
            toast.success(`${data.attachments.length} file(s) attached`);
          },
          onError: (error) => {
            toast.error(`Upload failed: ${error.message}`);
          },
        }
      );
    },
    [draftId, setDraftId, uploadMutation, addAttachments]
  );

  const handleToggleHtml = useCallback(() => {
    if (isHtml) {
      // HTML → plain text: strip tags
      setField("body", stripHtml(body));
    } else {
      // Plain text → HTML: wrap lines in <p> tags
      const html = body
        .split("\n")
        .map((line) => `<p>${line || "<br>"}</p>`)
        .join("");
      setField("body", html);
    }
    setIsHtml(!isHtml);
  }, [isHtml, body, setField, setIsHtml]);

  const previewAttachment = previewAttId
    ? attachments.find((a) => a.id === previewAttId)
    : null;

  return (
    <>
      <Dialog.Root
        open={isOpen}
        onOpenChange={(open) => {
          if (!open) {
            handleSaveAndClose();
          }
        }}
      >
        <Dialog.Portal forceMount>
          <AnimatePresence>
            {isOpen ? (
              <Fragment key="compose-dialog-open">
                {/* Only show scrim when expanded - collapsed bottom sheet has no overlay */}
                {(!isMobile || expanded) && (
                  <Dialog.Overlay forceMount asChild>
                    <AnimatedDiv
                      data-testid="compose-dialog-overlay-transition"
                      variants={overlayMotionProps}
                      initial="initial"
                      animate="animate"
                      exit="exit"
                      className="fixed inset-0 z-40 bg-black/40"
                    />
                  </Dialog.Overlay>
                )}
                <Dialog.Content forceMount asChild onOpenAutoFocus={(e) => e.preventDefault()}>
                  <AnimatedDiv
                    data-testid="compose-dialog-content-transition"
                    variants={contentMotionProps}
                    initial="initial"
                    animate="animate"
                    exit="exit"
                    className={cn(
                      "fixed z-50 flex flex-col overflow-hidden border border-border bg-background shadow-2xl",
                      isMobile
                        ? expanded
                          ? "inset-0 rounded-none"
                          : "inset-x-0 bottom-0 rounded-t-xl max-h-[45dvh]"
                        : expanded
                          ? "inset-4 rounded-xl sm:left-14"
                          : "inset-x-4 bottom-4 top-auto rounded-xl mx-auto max-h-[80vh] w-full max-w-2xl sm:inset-x-auto sm:bottom-8 sm:ml-14"
                    )}
                    style={isMobile ? { paddingBottom: "env(safe-area-inset-bottom)" } : undefined}
                    onKeyDown={handleKeyDown}
                    onDragEnter={handleDragEnter}
                    onDragLeave={handleDragLeave}
                    onDragOver={handleDragOver}
                    onDrop={handleDrop}
                  >
            {/* Drop overlay */}
            {isDragging && (
              <div className="absolute inset-0 z-10 flex flex-col items-center justify-center rounded-xl bg-background/90 backdrop-blur-sm">
                <Upload className="size-10 text-primary" />
                <p className="mt-3 text-sm font-medium text-foreground">
                  Drop files to attach
                </p>
                <p className="mt-1 text-xs text-muted-foreground">
                  Files will be uploaded as attachments
                </p>
              </div>
            )}

            {/* PGP passphrase — inline overlay, avoids framer-motion stacking-context issues with fixed positioning */}
            {showPgpPassphraseModal && (
              <div className="absolute inset-0 z-20 flex items-center justify-center rounded-xl bg-black/40 backdrop-blur-sm">
                <div className="w-full max-w-sm mx-6 rounded-lg border border-border bg-background p-6 shadow-xl">
                  <div className="flex items-center gap-2 mb-4">
                    {pgpMode === "encrypt" ? <Lock className="size-4" /> : <ShieldCheck className="size-4" />}
                    <h2 className="text-base font-semibold">
                      {pgpMode === "encrypt" ? "Encrypt message" : "Sign message"}
                    </h2>
                  </div>
                  <p className="text-sm text-muted-foreground mb-4">
                    Enter your key passphrase to {pgpMode === "encrypt" ? "encrypt" : "sign"} this message.
                  </p>
                  <input
                    type="password"
                    placeholder="Key passphrase"
                    value={pgpPassphrase}
                    onChange={(e) => setPgpPassphrase(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter" && pgpPassphrase) handlePgpSend(pgpPassphrase); }}
                    autoFocus
                    className="w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary mb-4"
                  />
                  <div className="flex gap-2">
                    <button
                      type="button"
                      onClick={() => handlePgpSend(pgpPassphrase)}
                      disabled={!pgpPassphrase || pgpSending}
                      className="inline-flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50 transition-colors"
                    >
                      {pgpSending && <Loader2 className="size-3.5 animate-spin" />}
                      {pgpMode === "encrypt" ? "Encrypt & Send" : "Sign & Send"}
                    </button>
                    <button
                      type="button"
                      onClick={() => { setShowPgpPassphraseModal(false); setPgpPassphrase(""); }}
                      className="rounded-md px-3 py-1.5 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              </div>
            )}

            {/* Missing PGP keys confirmation — inline overlay so it renders above compose without portal/stacking-context issues */}
            {missingKeysPending !== null && (
              <div className="absolute inset-0 z-20 flex items-center justify-center rounded-xl bg-black/40 backdrop-blur-sm">
                <div className="w-full max-w-sm mx-6 rounded-xl border border-border bg-background p-6 shadow-2xl">
                  <h2 className="text-base font-semibold">No key found for some recipients</h2>
                  <p className="mt-2 text-sm text-muted-foreground">
                    The following recipients have no PGP key and will not be able to decrypt this message:
                  </p>
                  <ul className="mt-3 max-h-40 space-y-1 overflow-y-auto">
                    {missingKeysPending.recipients.map((r) => (
                      <li key={r} className="text-sm font-medium">{r}</li>
                    ))}
                  </ul>
                  <div className="mt-6 flex justify-end gap-3">
                    <button
                      type="button"
                      onClick={() => setMissingKeysPending(null)}
                      className="rounded-lg px-4 py-2 text-sm font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
                    >
                      Cancel
                    </button>
                    <button
                      type="button"
                      onClick={handleMissingKeysConfirm}
                      className="rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                    >
                      Send anyway
                    </button>
                  </div>
                </div>
              </div>
            )}

            {/* Mobile drag handle */}
            {isMobile && !expanded && (
              <div className="flex justify-center pb-1 pt-2">
                <div className="h-1 w-8 rounded-full bg-border" />
              </div>
            )}

            {/* Header */}
            <div
              className="flex items-center justify-between border-b border-border px-4 py-3"
              onClick={isMobile && !expanded ? () => setExpanded(true) : undefined}
              style={isMobile && !expanded ? { cursor: "pointer" } : undefined}
            >
              <div className="flex items-center gap-2">
                <Dialog.Title className="text-sm font-semibold">
                  {mode === "reply" ? "Reply" : mode === "forward" ? "Forward" : "New Message"}
                </Dialog.Title>
                {hasContent() && computeHash() !== lastSavedHash && !draftSaved && (
                  <span className="size-1.5 rounded-full bg-muted-foreground/50" title="Unsaved changes" />
                )}
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={(e) => { e.stopPropagation(); setExpanded((v) => !v); }}
                  className="hidden md:flex rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                  title={expanded ? "Minimize" : "Maximize"}
                >
                  {expanded ? (
                    <Minimize2 className="size-4" />
                  ) : (
                    <Maximize2 className="size-4" />
                  )}
                </button>
                {isMobile && expanded && (
                  <button
                    onClick={() => setExpanded(false)}
                    className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                    title="Collapse"
                  >
                    <Minimize2 className="size-4" />
                  </button>
                )}
                <Dialog.Close asChild>
                  <button
                    className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                    title="Close"
                  >
                    <X className="size-4" />
                  </button>
                </Dialog.Close>
              </div>
            </div>

            {/* Fields */}
            <div className="flex flex-col border-b border-border">
              {identities && identities.length > 1 && (
                <div className="flex items-center border-b border-border/50 px-4">
                  <label className="w-12 shrink-0 text-xs text-muted-foreground">
                    From
                  </label>
                  <select
                    value={fromIdentityId ?? ""}
                    onChange={(e) => {
                      const newId = e.target.value ? Number(e.target.value) : null;
                      setFromIdentityId(newId);
                      // Swap signature when identity changes
                      if (identities && signatureEnabled) {
                        const newIdentity = identities.find((i) => i.id === newId);
                        const newSig = newIdentity?.signature_html || "";
                        setSignatureHtml(newSig);
                        setField("body", replaceSignatureInBody(body, newSig));
                      }
                    }}
                    className="flex-1 bg-transparent py-2 text-sm outline-none"
                  >
                    {identities.map((identity) => (
                      <option key={identity.id} value={identity.id}>
                        {identity.display_name
                          ? `${identity.display_name} <${identity.email}>`
                          : identity.email}
                      </option>
                    ))}
                  </select>
                </div>
              )}
              <div className="flex items-center border-b border-border/50 px-4">
                <label className="w-12 shrink-0 text-xs text-muted-foreground">
                  To
                </label>
                <RecipientInput
                  inputRef={toInputRef}
                  value={to}
                  onChange={(v) => setField("to", v)}
                  placeholder="Recipients"
                />
                <div className="ml-2 flex items-center gap-1">
                  {!showCc && (
                    <button
                      className="text-xs text-muted-foreground hover:text-foreground"
                      onClick={() => setShowCc(true)}
                    >
                      Cc
                    </button>
                  )}
                  {!showBcc && (
                    <button
                      className="text-xs text-muted-foreground hover:text-foreground"
                      onClick={() => setShowBcc(true)}
                    >
                      Bcc
                    </button>
                  )}
                </div>
              </div>

              {showCc && (
                <div className="flex items-center border-b border-border/50 px-4">
                  <label className="w-12 shrink-0 text-xs text-muted-foreground">
                    Cc
                  </label>
                  <RecipientInput
                    value={cc}
                    onChange={(v) => setField("cc", v)}
                  />
                </div>
              )}

              {showBcc && (
                <div className="flex items-center border-b border-border/50 px-4">
                  <label className="w-12 shrink-0 text-xs text-muted-foreground">
                    Bcc
                  </label>
                  <RecipientInput
                    value={bcc}
                    onChange={(v) => setField("bcc", v)}
                  />
                </div>
              )}

              <div className="flex items-center px-4">
                <label className="w-12 shrink-0 text-xs text-muted-foreground">
                  Subject
                </label>
                <input
                  type="text"
                  value={subject}
                  onChange={(e) => setField("subject", e.target.value)}
                  className="flex-1 bg-transparent py-2 text-sm outline-none placeholder:text-muted-foreground/50"
                />
              </div>
            </div>

            {/* Recipient count warning */}
            {countRecipients(to, cc, bcc) > 10 && (
              <div className="flex items-center gap-2 border-b border-yellow-300/50 bg-yellow-50 px-4 py-2 dark:border-yellow-700/50 dark:bg-yellow-950/30">
                <AlertTriangle className="size-4 shrink-0 text-yellow-600 dark:text-yellow-500" />
                <span className="text-xs text-yellow-700 dark:text-yellow-400">
                  You are sending to more than 10 recipients.
                </span>
              </div>
            )}

            {/* Attachments */}
            {attachments.length > 0 && (
              <div className="flex max-h-24 flex-wrap gap-2 overflow-y-auto border-b border-border px-4 py-2">
                {attachments.map((att) => (
                  <div
                    key={att.id}
                    className="flex items-center gap-1.5 rounded-md border border-border bg-accent/50 px-2 py-1 text-xs"
                  >
                    <button
                      onClick={() => setPreviewAttId(att.id)}
                      className="flex items-center gap-1.5 hover:text-foreground"
                      title="Preview"
                    >
                      <Paperclip className="size-3 shrink-0 text-muted-foreground" />
                      <span
                        className="max-w-[150px] truncate"
                        title={att.filename}
                      >
                        {att.filename}
                      </span>
                      <span className="text-muted-foreground">
                        ({formatFileSize(att.size)})
                      </span>
                    </button>
                    <button
                      onClick={() => handleRemoveAttachment(att.id)}
                      className="ml-0.5 rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                      title="Remove attachment"
                    >
                      <X className="size-3" />
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Body */}
            <div data-compose-body className="flex flex-1 flex-col min-h-0">
              {isHtml ? (
                <RichTextEditor
                  content={body}
                  onChange={(html) => setField("body", html)}
                  onImageUpload={handleImageUpload}
                  placeholder="Write your message..."
                  className="flex-1 overflow-auto"
                />
              ) : (
                <textarea
                  value={body}
                  onChange={(e) => setField("body", e.target.value)}
                  placeholder="Write your message..."
                  className="flex-1 resize-none overflow-auto bg-transparent px-4 py-3 text-sm outline-none placeholder:text-muted-foreground/50"
                />
              )}
            </div>

            {/* Collapsed quoted content for replies - sits inside the body area */}
            {(quotedHtml || quotedText) && (
              <div className="px-4 pb-2">
                <button
                  type="button"
                  onClick={() => setQuoteExpanded((v) => !v)}
                  className="rounded px-1.5 py-0.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  aria-expanded={quoteExpanded}
                  aria-label={quoteExpanded ? "Hide quoted message" : "Show quoted message"}
                >
                  <span className="tracking-widest">···</span>
                </button>
                <div
                  className={cn(
                    "overflow-hidden",
                    shouldAnimate && "transition-[max-height]",
                    quoteExpanded ? "max-h-48" : "max-h-0",
                  )}
                  style={shouldAnimate ? { transitionDuration: `${motionTokens.duration.normal * 1000}ms` } : undefined}
                >
                  <div className="mt-2 overflow-y-auto border-l-2 border-border pl-3 text-sm text-muted-foreground" style={{ maxHeight: "11rem" }}>
                    {isHtml && quotedHtml ? (
                      <div
                        className="prose prose-sm max-w-none dark:prose-invert"
                        dangerouslySetInnerHTML={{ __html: quotedHtml }}
                      />
                    ) : (
                      <pre className="whitespace-pre-wrap font-sans text-xs">{quotedText}</pre>
                    )}
                  </div>
                </div>
              </div>
            )}

            {/* Footer */}
            <div className="flex items-center justify-between border-t border-border px-4 py-3">
              <div className="flex items-center gap-2">
                <button
                  onClick={handleSend}
                  disabled={
                    sendMutation.isPending ||
                    (!to.trim() && !cc.trim() && !bcc.trim())
                  }
                  className={cn(
                    "inline-flex items-center gap-2 rounded-lg px-4 py-2 text-sm font-medium transition-colors",
                    "bg-primary text-primary-foreground hover:bg-primary/90",
                    "disabled:cursor-not-allowed disabled:opacity-50"
                  )}
                >
                  {shouldAnimate ? (
                    <AnimatePresence mode="wait" initial={false}>
                      <motion.span
                        key={sendMutation.isPending ? "pending" : "idle"}
                        data-testid="compose-send-feedback-transition"
                        data-motion-props={serializedSendFeedbackMotionProps}
                        initial="initial"
                        animate="animate"
                        exit="exit"
                        variants={sendFeedbackMotionProps}
                        className="inline-flex items-center gap-2"
                      >
                        {sendMutation.isPending ? (
                          <>
                            <Loader2 className="size-4 animate-spin" />
                            Sending...
                          </>
                        ) : (
                          <>
                            <Send className="size-4" />
                            Send
                          </>
                        )}
                      </motion.span>
                    </AnimatePresence>
                  ) : sendMutation.isPending ? (
                    <>
                      <Loader2 className="size-4 animate-spin" />
                      Sending...
                    </>
                  ) : (
                    <>
                      <Send className="size-4" />
                      Send
                    </>
                  )}
                </button>
                <button
                  onClick={handleAttachFiles}
                  disabled={uploadMutation.isPending}
                  className="rounded-lg p-2 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                  title="Attach files"
                >
                  {uploadMutation.isPending ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Paperclip className="size-4" />
                  )}
                </button>
                <button
                  onClick={() => saveDraft(true)}
                  disabled={saveDraftMutation.isPending}
                  className="rounded-lg p-2 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"
                  title="Save draft (Ctrl+S)"
                >
                  {saveDraftMutation.isPending ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Save className="size-4" />
                  )}
                </button>
                <button
                  onClick={handleToggleHtml}
                  className={cn(
                    "rounded-lg p-2 hover:bg-accent hover:text-foreground",
                    isHtml ? "text-muted-foreground" : "text-primary"
                  )}
                  title={isHtml ? "Switch to plain text" : "Switch to rich text"}
                >
                  {isHtml ? (
                    <FileText className="size-4" />
                  ) : (
                    <Code className="size-4" />
                  )}
                </button>
                <button
                  onClick={() => {
                    const newEnabled = !signatureEnabled;
                    setSignatureEnabled(newEnabled);
                    if (newEnabled) {
                      // Re-inject signature
                      if (signatureHtml) {
                        setField("body", injectSignature(removeSignatureFromBody(body), signatureHtml));
                      }
                    } else {
                      // Remove signature from body
                      setField("body", removeSignatureFromBody(body));
                    }
                  }}
                  className={cn(
                    "rounded-lg p-2 hover:bg-accent hover:text-foreground",
                    signatureEnabled ? "text-primary" : "text-muted-foreground"
                  )}
                  title={signatureEnabled ? "Remove signature" : "Add signature"}
                >
                  <PenLine className="size-4" />
                </button>
                {hasPgpKeys && (
                  <button
                    type="button"
                    onClick={() => setPgpMode(pgpMode === "off" ? "sign" : pgpMode === "sign" ? "encrypt" : "off")}
                    className={cn(
                      "rounded-lg p-2 hover:bg-accent hover:text-foreground",
                      pgpMode === "off" ? "text-muted-foreground" : "text-primary"
                    )}
                    title={pgpMode === "off" ? "Encryption off" : pgpMode === "sign" ? "Sign only" : "Encrypt & sign"}
                  >
                    {pgpMode === "encrypt" ? (
                      <Lock className="size-4" />
                    ) : pgpMode === "sign" ? (
                      <ShieldCheck className="size-4" />
                    ) : (
                      <LockOpen className="size-4" />
                    )}
                  </button>
                )}
                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={handleFileSelected}
                />
              </div>
              <div className="flex items-center gap-2">
                {draftSaved && (
                  <span className="text-xs text-muted-foreground">
                    Draft saved
                  </span>
                )}
                <button
                  onClick={handleDiscard}
                  className="rounded-lg px-3 py-2 text-sm text-muted-foreground hover:bg-accent hover:text-foreground"
                >
                  Discard
                </button>
              </div>
            </div>
                  </AnimatedDiv>
                </Dialog.Content>
              </Fragment>
            ) : null}
          </AnimatePresence>
        </Dialog.Portal>
      </Dialog.Root>

      <DiscardAlertDialog
        open={showDiscardAlert}
        onOpenChange={setShowDiscardAlert}
        onConfirm={confirmDiscard}
      />


      {/* Attachment preview dialog */}
      {previewAttachment && draftId && (
        <AttachmentPreviewDialog
          attachment={{
            id: previewAttachment.id,
            filename: previewAttachment.filename,
            contentType: previewAttachment.contentType,
            size: previewAttachment.size,
          }}
          previewUrl={`${process.env.NEXT_PUBLIC_BASE_PATH || ""}/api/drafts/${draftId}/attachments/${previewAttachment.id}/content`}
          onClose={() => setPreviewAttId(null)}
        />
      )}

    </>
  );
}
