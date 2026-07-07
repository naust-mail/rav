"use client";

import { useState, useEffect, useCallback, useMemo } from "react";
import { Dialog } from "radix-ui";
import { AnimatePresence } from "framer-motion";
import {
  Paperclip,
  X,
  Download,
  ChevronLeft,
  ChevronRight,
  FileText,
  FileAudio,
  FileVideo,
  File,
  CalendarDays,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { createFadeSlideVariants, createScaleFadeVariants } from "@/lib/motion/variants";
import { AnimatedDiv } from "@/lib/motion/AnimatedDiv";
import { useUiStore } from "@/stores/useUiStore";
import { formatFileSize } from "./utils";
import { IcsPreview } from "./IcsPreview";
import type { Attachment } from "@/types/message";

function isCalendarType(ct: string): boolean {
  return ct === "text/calendar" || ct === "application/ics";
}

function isPdfType(ct: string): boolean {
  return ct.toLowerCase() === "application/pdf";
}

function isTextType(ct: string): boolean {
  return ct.toLowerCase().startsWith("text/") && !isCalendarType(ct);
}

function isPgpType(ct: string): boolean {
  return ct.toLowerCase().startsWith("application/pgp");
}

function isVideoType(ct: string): boolean {
  return ct.toLowerCase().startsWith("video/");
}

function isAudioType(ct: string): boolean {
  return ct.toLowerCase().startsWith("audio/");
}

function ThumbnailIcon({ contentType }: { contentType: string }) {
  if (isCalendarType(contentType)) return <CalendarDays className="size-6 text-muted-foreground" />;
  if (isPdfType(contentType) || isTextType(contentType) || isPgpType(contentType)) return <FileText className="size-6 text-muted-foreground" />;
  if (isVideoType(contentType)) return <FileVideo className="size-6 text-muted-foreground" />;
  if (isAudioType(contentType)) return <FileAudio className="size-6 text-muted-foreground" />;
  return <File className="size-6 text-muted-foreground" />;
}

function TextPreview({ url }: { url: string }) {
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState(false);

  useEffect(() => {
    fetch(url)
      .then((r) => r.text())
      .then(setText)
      .catch(() => setError(true));
  }, [url]);

  if (error) return <p className="text-sm text-muted-foreground">Failed to load preview.</p>;
  if (text === null) return <p className="text-sm text-muted-foreground">Loading...</p>;

  // React renders {text} as a text node - no HTML escaping needed, no XSS risk.
  return (
    <pre className="h-full w-full overflow-auto rounded-md bg-muted p-4 font-mono text-xs leading-relaxed text-foreground">
      {text}
    </pre>
  );
}

interface AttachmentPreviewerProps {
  attachments: Attachment[];
  baseUrl: string;
  accountId: string | null;
  initialIndex: number;
  onClose: () => void;
}

export function AttachmentPreviewer({
  attachments,
  baseUrl,
  accountId,
  initialIndex,
  onClose,
}: AttachmentPreviewerProps) {
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);
  const overlayMotionProps = useMemo(() => createFadeSlideVariants(effectiveAnimationMode, "y"), [effectiveAnimationMode]);
  const contentMotionProps = useMemo(() => createScaleFadeVariants(effectiveAnimationMode), [effectiveAnimationMode]);
  const [index, setIndex] = useState(initialIndex);
  const att = attachments[index];
  const buildUrl = (attId: string) => {
    const path = `${baseUrl}/${attId}`;
    return accountId ? `${path}?account_id=${encodeURIComponent(accountId)}` : path;
  };
  const url = buildUrl(att.id);

  const goPrev = useCallback(() => setIndex((i) => Math.max(0, i - 1)), [setIndex]);
  const goNext = useCallback(
    () => setIndex((i) => Math.min(attachments.length - 1, i + 1)),
    [attachments.length, setIndex],
  );

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowLeft") goPrev();
      else if (e.key === "ArrowRight") goNext();
      else if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [goPrev, goNext, onClose]);

  return (
    <Dialog.Root open onOpenChange={(open) => !open && onClose()}>
      <Dialog.Portal forceMount>
        <AnimatePresence>
          <Dialog.Overlay key="overlay" asChild>
            <AnimatedDiv
              data-testid="reading-pane-attachment-overlay-transition"
              variants={overlayMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              className="fixed inset-0 z-50 bg-black/70"
            />
          </Dialog.Overlay>
          <Dialog.Content key="content" asChild>
            <AnimatedDiv
              data-testid="reading-pane-attachment-content-transition"
              variants={contentMotionProps}
              initial="initial"
              animate="animate"
              exit="exit"
              className="fixed inset-4 z-50 flex flex-col rounded-xl border border-border bg-background shadow-2xl"
            >
          {/* Header */}
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <Dialog.Title className="flex items-center gap-2 text-sm font-semibold">
              <Paperclip className="size-4 text-muted-foreground" />
              <span className="max-w-[400px] truncate">
                {att.filename ?? "Attachment"}
              </span>
              <span className="text-xs font-normal text-muted-foreground">
                ({formatFileSize(att.size)})
              </span>
              {attachments.length > 1 && (
                <span className="text-xs font-normal text-muted-foreground">
                  — {index + 1} of {attachments.length}
                </span>
              )}
            </Dialog.Title>
            <div className="flex items-center gap-1">
              {attachments.length > 1 && (
                <>
                  <button
                    onClick={goPrev}
                    disabled={index === 0}
                    className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-30"
                    title="Previous"
                  >
                    <ChevronLeft className="size-4" />
                  </button>
                  <button
                    onClick={goNext}
                    disabled={index === attachments.length - 1}
                    className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-30"
                    title="Next"
                  >
                    <ChevronRight className="size-4" />
                  </button>
                </>
              )}
              <a
                href={url}
                download={att.filename ?? undefined}
                className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground"
                title="Download"
              >
                <Download className="size-4" />
              </a>
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

          {/* Thumbnail strip */}
          {attachments.length > 1 && (
            <div className="flex shrink-0 gap-2 overflow-x-auto border-b border-border bg-muted/30 px-4 py-2">
              {attachments.map((thumb, i) => {
                const thumbUrl = buildUrl(thumb.id);
                const isActive = i === index;
                return (
                  <button
                    key={`${thumb.id}-${i}`}
                    onClick={() => setIndex(i)}
                    className={cn(
                      "flex size-14 shrink-0 items-center justify-center overflow-hidden rounded-md border-2 transition-colors",
                      isActive
                        ? "border-primary bg-accent"
                        : "border-transparent bg-muted hover:border-muted-foreground/30",
                    )}
                    title={thumb.filename ?? `Attachment ${i + 1}`}
                  >
                    {thumb.content_type.startsWith("image/") ? (
                      // eslint-disable-next-line @next/next/no-img-element -- blob URL thumbnail, not optimizable
                      <img
                        src={thumbUrl}
                        alt={thumb.filename ?? ""}
                        className="size-full object-cover"
                      />
                    ) : (
                      <ThumbnailIcon contentType={thumb.content_type} />
                    )}
                  </button>
                );
              })}
            </div>
          )}

          {/* Preview content */}
          <div className="flex flex-1 items-center justify-center overflow-auto p-4">
            {att.content_type.startsWith("image/") ? (
              // eslint-disable-next-line @next/next/no-img-element -- blob URL preview, not optimizable
              <img
                src={url}
                alt={att.filename ?? "Attachment"}
                className="max-h-full max-w-full object-contain"
              />
            ) : isPdfType(att.content_type) ? (
              <iframe
                src={url}
                className="h-full w-full border-none"
                title={att.filename ?? "PDF"}
                sandbox="allow-scripts allow-same-origin"
              />
            ) : isCalendarType(att.content_type) ? (
              <IcsPreview url={url} filename={att.filename} />
            ) : isTextType(att.content_type) || isPgpType(att.content_type) ? (
              <TextPreview key={url} url={url} />
            ) : isVideoType(att.content_type) ? (
              <video src={url} controls className="max-h-full max-w-full rounded-md" />
            ) : isAudioType(att.content_type) ? (
              <audio src={url} controls className="w-full max-w-md" />
            ) : (
              <div className="flex flex-col items-center gap-4 text-center">
                <Paperclip className="size-12 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">
                  Preview not available for this file type
                </p>
                <a
                  href={url}
                  download={att.filename ?? undefined}
                  className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                >
                  <Download className="size-4" />
                  Download
                </a>
              </div>
            )}
          </div>
            </AnimatedDiv>
          </Dialog.Content>
        </AnimatePresence>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
