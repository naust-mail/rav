"use client";

import { useState, useMemo } from "react";
import {
  Reply,
  ReplyAll,
  Forward,
  Trash2,
  Archive,
  Star,
  Mail,
  MailOpen,
  ShieldOff,
  ShieldCheck,
  Loader2,
  Printer,
} from "lucide-react";
import { AnimatePresence, motion } from "framer-motion";
import { toast } from "sonner";
import { useUiStore } from "@/stores/useUiStore";
import {
  useMessage,
  useUpdateFlags,
  useMoveMessage,
  useDeleteMessage,
} from "@/hooks/useMessages";
import { MoveToFolderMenu } from "./MoveToFolderMenu";
import { TagPicker } from "./TagPicker";
import { Button } from "@/components/ui/button";
import { ActionTooltip, ActionTooltipProvider } from "./ActionTooltip";
import { useComposeStore } from "@/stores/useComposeStore";
import { useReportSpam, useReportHam } from "@/hooks/useSpamReport";
import { useAuthStore } from "@/stores/useAuthStore";
import {
  extractHeader,
  buildReplySubject,
  buildForwardSubject,
  buildReplyQuoteHtml,
  buildReplyQuoteText,
  buildForwardBody,
  buildForwardBodyHtml,
  buildReferences,
} from "@/lib/email-utils";
import type { EmailAddress } from "@/types/message";
import { useIdentities } from "@/hooks/useIdentities";
import type { Identity } from "@/types/identity";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";

/** Find the identity whose email matches one of the To/CC addresses. */
function findMatchingIdentity(
  identities: Identity[] | undefined,
  toAddresses: EmailAddress[],
  ccAddresses: EmailAddress[],
): number | null {
  if (!identities || identities.length === 0) return null;
  const allRecipientEmails = [...toAddresses, ...ccAddresses].map((a) =>
    a.address.toLowerCase(),
  );
  const match = identities.find((i) =>
    allRecipientEmails.includes(i.email.toLowerCase()),
  );
  return match?.id ?? null;
}

function formatAddressList(addresses: EmailAddress[]): string {
  return addresses
    .map((a) => (a.name ? `${a.name} <${a.address}>` : a.address))
    .join(", ");
}

export function MessageActionBar() {
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedMessageUid = useUiStore((s) => s.selectedMessageUid);
  const updateFlags = useUpdateFlags();
  const moveMessage = useMoveMessage();
  const deleteMessage = useDeleteMessage();
  const reportSpam = useReportSpam();
  const reportHam = useReportHam();

  const { data } = useMessage(activeFolder, selectedMessageUid ?? 0);
  const { data: identities } = useIdentities();

  const disabled = !data;
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const shouldAnimate = effectiveAnimationMode !== "off";
  const barMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const feedbackMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const serializedFeedbackMotionProps = useMemo(() => JSON.stringify(feedbackMotionProps), [feedbackMotionProps]);
  const [actionFeedback, setActionFeedback] = useState<"delete" | "archive" | "move" | null>(null);

  const isSeen = data?.flags.includes("\\Seen") ?? false;
  const isFlagged = data?.flags.includes("\\Flagged") ?? false;

  // Reply All is redundant for 1-to-1 conversations (no other recipients besides sender + me)
  const isDirectConversation = (() => {
    if (!data) return false;
    const myEmail = useAuthStore.getState().activeAccount()?.email ?? "";
    const otherRecipients = [...data.to_addresses, ...data.cc_addresses].filter(
      (a) =>
        a.address.toLowerCase() !== myEmail.toLowerCase() &&
        a.address.toLowerCase() !== data.from_address.toLowerCase(),
    );
    return otherRecipients.length === 0;
  })();

  const handleReply = () => {
    if (!data) return;
    const messageId = extractHeader(data.raw_headers, "Message-ID");
    const refs = extractHeader(data.raw_headers, "References");
    const matchedId = findMatchingIdentity(identities, data.to_addresses, data.cc_addresses);
    const hasHtml = !!(data.html && data.html.trim());
    useComposeStore.getState().openReply({
      to: data.from_address,
      cc: "",
      subject: buildReplySubject(data.subject),
      body: hasHtml ? "<p><br></p>" : "",
      quotedHtml: hasHtml ? buildReplyQuoteHtml(data.html!, data.from_address, data.date) : null,
      quotedText: buildReplyQuoteText(data.text, data.from_address, data.date),
      inReplyTo: messageId,
      references: buildReferences(refs, messageId),
      fromIdentityId: matchedId,
      isHtml: hasHtml,
    });
  };

  const handleReplyAll = () => {
    if (!data) return;
    const myEmail = useAuthStore.getState().activeAccount()?.email ?? "";
    const messageId = extractHeader(data.raw_headers, "Message-ID");
    const refs = extractHeader(data.raw_headers, "References");
    const replyTo = data.from_address;
    const allRecipients = [
      ...data.to_addresses,
      ...data.cc_addresses,
    ].filter(
      (a) =>
        a.address.toLowerCase() !== myEmail.toLowerCase() &&
        a.address.toLowerCase() !== data.from_address.toLowerCase(),
    );
    const ccList = allRecipients.map((a) => a.address).join(", ");
    const matchedId = findMatchingIdentity(identities, data.to_addresses, data.cc_addresses);
    const hasHtml = !!(data.html && data.html.trim());
    useComposeStore.getState().openReply({
      to: replyTo,
      cc: ccList,
      subject: buildReplySubject(data.subject),
      body: hasHtml ? "<p><br></p>" : "",
      quotedHtml: hasHtml ? buildReplyQuoteHtml(data.html!, data.from_address, data.date) : null,
      quotedText: buildReplyQuoteText(data.text, data.from_address, data.date),
      inReplyTo: messageId,
      references: buildReferences(refs, messageId),
      fromIdentityId: matchedId,
      isHtml: hasHtml,
    });
  };

  const handleForward = () => {
    if (!data) return;
    const toList = formatAddressList(data.to_addresses);
    const hasHtml = !!(data.html && data.html.trim());
    useComposeStore.getState().openForward({
      subject: buildForwardSubject(data.subject),
      body: hasHtml
        ? buildForwardBodyHtml(data.html!, data.from_address, data.date, data.subject, toList)
        : buildForwardBody(data.text, data.from_address, data.date, data.subject, toList),
      isHtml: hasHtml,
    });
  };

  const handleDelete = () => {
    if (!data) return;
    setActionFeedback("delete");
    const isPermanent = activeFolder === "Trash";
    const label = isPermanent ? "Permanently deleted" : "Moved to trash";
    const uid = data.uid;
    const folder = activeFolder;
    let cancelled = false;

    toast(label, {
      duration: 5000,
      action: {
        label: "Undo",
        onClick: () => { cancelled = true; setActionFeedback(null); },
      },
    });

    setTimeout(() => {
      if (cancelled) return;
      if (isPermanent) {
        deleteMessage.mutate(
          { folder, uid },
          { onSettled: () => setActionFeedback(null) },
        );
      } else {
        moveMessage.mutate(
          { fromFolder: folder, toFolder: "Trash", uid },
          { onSettled: () => setActionFeedback(null) },
        );
      }
    }, 5000);
  };

  const handleArchive = () => {
    if (!data) return;
    setActionFeedback("archive");
    const uid = data.uid;
    const folder = activeFolder;
    let cancelled = false;

    toast("Archived", {
      duration: 5000,
      action: {
        label: "Undo",
        onClick: () => { cancelled = true; setActionFeedback(null); },
      },
    });

    setTimeout(() => {
      if (cancelled) return;
      moveMessage.mutate(
        { fromFolder: folder, toFolder: "Archive", uid },
        { onSettled: () => setActionFeedback(null) },
      );
    }, 5000);
  };

  const handleJunk = () => {
    if (!data) return;
    const uid = data.uid;
    const folder = activeFolder;
    reportSpam.mutate({ folder, uid });
    moveMessage.mutate({ fromFolder: folder, toFolder: "Spam", uid });
  };

  const handleNotJunk = () => {
    if (!data) return;
    const uid = data.uid;
    reportHam.mutate({ folder: "Spam", uid });
    moveMessage.mutate({ fromFolder: "Spam", toFolder: "INBOX", uid });
  };

  const handleToggleStar = () => {
    if (!data) return;
    updateFlags.mutate({
      folder: activeFolder,
      uid: data.uid,
      flags: ["\\Flagged"],
      add: !isFlagged,
    });
  };

  const handleToggleRead = () => {
    if (!data) return;
    updateFlags.mutate({
      folder: activeFolder,
      uid: data.uid,
      flags: ["\\Seen"],
      add: !isSeen,
    });
  };

  return (
    <ActionTooltipProvider>
    <AnimatedDiv
      data-testid="message-action-bar-transition"
      variants={barMotionProps}
      initial="initial"
      animate="animate"
      exit="exit"
      className="flex shrink-0 flex-wrap items-center gap-0.5 border-b border-border px-2 py-1"
    >
      {/* Reply */}
      <ActionTooltip label="Reply">
        <Button aria-label="Reply" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleReply}>
          <Reply className="size-4" />
          <span className="hidden xl:inline">Reply</span>
        </Button>
      </ActionTooltip>

      {/* Reply All — disabled for 1-to-1 conversations */}
      <ActionTooltip label="Reply all">
        <Button aria-label="Reply all" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled || isDirectConversation} onClick={handleReplyAll}>
          <ReplyAll className="size-4" />
          <span className="hidden xl:inline">Reply all</span>
        </Button>
      </ActionTooltip>

      {/* Forward */}
      <ActionTooltip label="Forward">
        <Button aria-label="Forward" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleForward}>
          <Forward className="size-4" />
          <span className="hidden xl:inline">Forward</span>
        </Button>
      </ActionTooltip>

      <div className="mx-0.5 h-5 w-px shrink-0 bg-border" />

      {/* Delete */}
      <ActionTooltip label={activeFolder === "Trash" ? "Delete permanently" : "Move to Trash"}>
      <Button aria-label={activeFolder === "Trash" ? "Delete permanently" : "Move to Trash"} variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleDelete}>
        {shouldAnimate ? (
          <AnimatePresence mode="wait" initial={false}>
            <motion.span
              key={actionFeedback === "delete" ? "delete-busy" : "delete-idle"}
              data-testid="message-action-delete-feedback-transition"
              data-motion-props={serializedFeedbackMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              variants={feedbackMotionProps}
              className="inline-flex"
            >
              {actionFeedback === "delete" ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Trash2 className="size-4" />
              )}
            </motion.span>
          </AnimatePresence>
        ) : (
          <Trash2 className="size-4" />
        )}
        <span className="hidden xl:inline">{activeFolder === "Trash" ? "Delete permanently" : "Move to Trash"}</span>
      </Button>
      </ActionTooltip>

      {/* Archive */}
      <ActionTooltip label="Archive">
      <Button aria-label="Archive" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled || activeFolder === "Archive"} onClick={handleArchive}>
        {shouldAnimate ? (
          <AnimatePresence mode="wait" initial={false}>
            <motion.span
              key={actionFeedback === "archive" ? "archive-busy" : "archive-idle"}
              data-testid="message-action-archive-feedback-transition"
              data-motion-props={serializedFeedbackMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              variants={feedbackMotionProps}
              className="inline-flex"
            >
              {actionFeedback === "archive" ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <Archive className="size-4" />
              )}
            </motion.span>
          </AnimatePresence>
        ) : (
          <Archive className="size-4" />
        )}
        <span className="hidden xl:inline">Archive</span>
      </Button>
      </ActionTooltip>

      {/* Move to */}
      {shouldAnimate ? (
        <AnimatePresence mode="wait" initial={false}>
          <motion.span
            key={actionFeedback === "move" ? "move-busy" : "move-idle"}
            data-testid="message-action-move-feedback-transition"
            data-motion-props={serializedFeedbackMotionProps}
            initial="initial"
            animate="animate"
            exit="exit"
            variants={feedbackMotionProps}
            className="inline-flex"
          >
            {actionFeedback === "move" ? (
              <Button variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled>
                <Loader2 className="size-4 animate-spin" />
                <span className="hidden xl:inline">Move to...</span>
              </Button>
            ) : disabled ? (
              <Button variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled>
                <span className="hidden xl:inline">Move to...</span>
              </Button>
            ) : (
              <MoveToFolderMenu
                currentFolder={activeFolder}
                onMove={(toFolder) => {
                  setActionFeedback("move");
                  moveMessage.mutate(
                    { fromFolder: activeFolder, toFolder, uid: data.uid },
                    { onSettled: () => setActionFeedback(null) },
                  );
                }}
              />
            )}
          </motion.span>
        </AnimatePresence>
      ) : disabled ? (
        <Button variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled>
          <span className="hidden xl:inline">Move to...</span>
        </Button>
      ) : (
        <MoveToFolderMenu
          currentFolder={activeFolder}
          onMove={(toFolder) => {
            moveMessage.mutate({ fromFolder: activeFolder, toFolder, uid: data.uid });
          }}
        />
      )}

      {/* Junk / Not Junk */}
      {activeFolder === "Spam" ? (
        <ActionTooltip label="Not junk - move to Inbox">
          <Button aria-label="Not junk" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleNotJunk}>
            <ShieldCheck className="size-4" />
            <span className="hidden xl:inline">Not Junk</span>
          </Button>
        </ActionTooltip>
      ) : (
        <ActionTooltip label="Mark as junk">
          <Button aria-label="Mark as junk" variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleJunk}>
            <ShieldOff className="size-4" />
            <span className="hidden xl:inline">Junk</span>
          </Button>
        </ActionTooltip>
      )}

      {/* Star/Unstar */}
      <ActionTooltip label={isFlagged ? "Unstar" : "Star"}>
      <Button aria-label={isFlagged ? "Unstar" : "Star"} variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleToggleStar}>
        {shouldAnimate ? (
          <AnimatePresence mode="wait" initial={false}>
            <motion.span
              key={isFlagged ? "flagged" : "unflagged"}
              data-testid="message-action-star-feedback-transition"
              data-motion-props={serializedFeedbackMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              variants={feedbackMotionProps}
              className="inline-flex"
            >
              {isFlagged ? (
                <Star className="size-4 fill-primary text-primary" />
              ) : (
                <Star className="size-4" />
              )}
            </motion.span>
          </AnimatePresence>
        ) : isFlagged ? (
          <Star className="size-4 fill-primary text-primary" />
        ) : (
          <Star className="size-4" />
        )}
        <span className="hidden xl:inline">{isFlagged ? "Unstar" : "Star"}</span>
      </Button>
      </ActionTooltip>

      {/* Mark read/unread */}
      <ActionTooltip label={isSeen ? "Mark as unread" : "Mark as read"}>
      <Button aria-label={isSeen ? "Mark as unread" : "Mark as read"} variant="ghost" size="sm" className="shrink-0 gap-1.5" disabled={disabled} onClick={handleToggleRead}>
        {shouldAnimate ? (
          <AnimatePresence mode="wait" initial={false}>
            <motion.span
              key={isSeen ? "seen" : "unseen"}
              data-testid="message-action-read-feedback-transition"
              data-motion-props={serializedFeedbackMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              variants={feedbackMotionProps}
              className="inline-flex"
            >
              {isSeen ? (
                <MailOpen className="size-4" />
              ) : (
                <Mail className="size-4" />
              )}
            </motion.span>
          </AnimatePresence>
        ) : isSeen ? (
          <MailOpen className="size-4" />
        ) : (
          <Mail className="size-4" />
        )}
        <span className="hidden xl:inline">{isSeen ? "Unread" : "Read"}</span>
      </Button>
      </ActionTooltip>

      {/* Print */}
      <ActionTooltip label="Print">
      <Button
        aria-label="Print"
        variant="ghost"
        size="sm"
        className="shrink-0 gap-1.5"
        disabled={disabled}
        onClick={() => {
          if (!data) return;
          const content = data.html || `<pre>${data.text}</pre>`;
          const win = window.open("", "_blank");
          if (!win) return;
          win.document.write(`<!DOCTYPE html><html><head><title>${data.subject}</title></head><body>${content}</body></html>`);
          win.document.close();
          win.focus();
          win.print();
        }}
      >
        <Printer className="size-4" />
        <span className="hidden xl:inline">Print</span>
      </Button>
      </ActionTooltip>

      {/* Tags */}
      {data && (
        <TagPicker folder={activeFolder} uid={data.uid} />
      )}
    </AnimatedDiv>
    </ActionTooltipProvider>
  );
}
