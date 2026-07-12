"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { PgpSendRequest } from "@/types/pgp";
import type { FolderId } from "@/types/folder";
import type { SendRequest } from "@/types/generated/SendRequest";
import type { SendResponse } from "@/types/generated/SendResponse";
import type { EnqueueResponse } from "@/types/generated/EnqueueResponse";
import type { ListResponse } from "@/types/generated/ListResponse";
import type { StatusResponse } from "@/types/generated/StatusResponse";
import { apiPost, apiGet, apiPostFormData, apiDelete } from "@/lib/api";

export type { OutboxEntry } from "@/types/generated/OutboxEntry";

type SendParams = {
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  htmlBody: string | null;
  inReplyTo: string | null;
  references: string | null;
  /** UUID of the draft this send originates from, if any. Used to clean up the IMAP copy after send. */
  draftId: string | null;
  fromIdentityId: number | null;
  pgp?: PgpSendRequest | null;
};

function toSendRequest(params: SendParams): SendRequest {
  return {
    to: parseRecipients(params.to),
    cc: parseRecipients(params.cc),
    bcc: parseRecipients(params.bcc),
    subject: params.subject,
    text_body: params.body,
    html_body: params.htmlBody,
    in_reply_to: params.inReplyTo,
    references: params.references,
    draft_id: params.draftId,
    from_identity_id: params.fromIdentityId,
    pgp: params.pgp ?? null,
  };
}

type UploadResponse = {
  attachments: {
    id: string;
    filename: string;
    content_type: string;
    size: number;
  }[];
};

type DeleteAttachmentResponse = {
  status: string;
};

/** Parameters for saving a draft. `uuid` is client-generated and stable across saves. */
type SaveDraftParams = {
  /** Client-generated UUID. Used as the path segment and embedded as Message-ID in IMAP. */
  uuid: string;
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  textBody: string;
  htmlBody: string | null;
  inReplyTo: string | null;
  references: string | null;
};

type SaveDraftResponse = {
  status: string;
};

/** One staged attachment for a draft, as returned by the backend. */
export type DraftAttachment = {
  id: string;
  filename: string;
  content_type: string;
  size: number;
  created_at: string;
};

type DraftAttachmentsResponse = {
  attachments: DraftAttachment[];
};

function parseRecipients(raw: string): string[] {
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/** Immediate send, bypassing the outbox/undo-delay queue. Not used by the
 * compose flow (which goes through `useEnqueueOutbox` instead so sends are
 * undoable and retryable) - kept for callers that want a direct send. */
export function useSendMessage() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (params: SendParams) =>
      apiPost<SendResponse>("/messages/send", toSendRequest(params)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

/** Queue a message for sending after the user's undo-send delay. Returns
 * immediately with the outbox entry id, used to undo via `useCancelOutbox`. */
export function useEnqueueOutbox() {
  return useMutation({
    mutationFn: (params: SendParams) =>
      apiPost<EnqueueResponse>("/outbox", toSendRequest(params)),
  });
}

/** List entries still scheduled or permanently failed. */
export function useOutboxList() {
  return useQuery({
    queryKey: ["outbox"],
    queryFn: () => apiGet<ListResponse>("/outbox"),
  });
}

/** Undo (while scheduled) or discard (while failed) a queued send. */
export function useCancelOutbox() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiDelete<StatusResponse>(`/outbox/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["outbox"] });
    },
  });
}

/** Requeue a permanently failed entry for an immediate retry. */
export function useRetryOutbox() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiPost<StatusResponse>(`/outbox/${id}/retry`, {}),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["outbox"] });
    },
  });
}

export function useUploadAttachment() {
  return useMutation({
    mutationFn: ({
      draftId,
      files,
    }: {
      draftId: string;
      files: File[];
    }) => {
      const formData = new FormData();
      for (const file of files) {
        formData.append("file", file);
      }
      return apiPostFormData<UploadResponse>(
        `/drafts/${draftId}/attachments`,
        formData,
      );
    },
  });
}

export function useDeleteAttachment() {
  return useMutation({
    mutationFn: ({
      draftId,
      attachmentId,
    }: {
      draftId: string;
      attachmentId: string;
    }) =>
      apiDelete<DeleteAttachmentResponse>(
        `/drafts/${draftId}/attachments/${attachmentId}`,
      ),
  });
}

/** Save draft body to IMAP via POST /drafts/{uuid}. The UUID travels in the path, not the body. */
export function useSaveDraft() {
  return useMutation({
    mutationFn: (params: SaveDraftParams) =>
      apiPost<SaveDraftResponse>(`/drafts/${params.uuid}`, {
        to: params.to,
        cc: params.cc,
        bcc: params.bcc,
        subject: params.subject,
        text_body: params.textBody,
        html_body: params.htmlBody,
        in_reply_to: params.inReplyTo,
        references: params.references,
      }),
  });
}

/** Delete/discard a draft: expunges from IMAP and removes staging record. */
export function useDeleteDraft() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (uuid: string) =>
      apiDelete<{ status: string }>(`/drafts/${uuid}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["messages"] });
      queryClient.invalidateQueries({ queryKey: ["folders"] });
    },
  });
}

/** Returned by GET /drafts/reply-for/:message_id when a reply draft exists. */
export type ReplyDraftRef = {
  uuid: string;
  imap_uid: number | null;
  /** Opaque, single-use token - see `FolderId`. Use directly, right away. */
  draft_folder_id: FolderId;
};

/**
 * Eagerly fetch the reply draft for a message as soon as it is opened.
 * Result is cached by React Query so Reply clicks read it synchronously.
 * Returns null when no draft exists (404) or messageId is empty.
 */
export function useReplyDraft(messageId: string | null) {
  return useQuery({
    queryKey: ["reply-draft", messageId],
    queryFn: () =>
      apiPost<ReplyDraftRef>(`/drafts/reply-for`, { message_id: messageId! }).catch(
        () => null,
      ),
    enabled: !!messageId,
    staleTime: 30_000,
  });
}

/** Fetch staged attachments for a draft UUID (used when reopening a draft from IMAP). */
export function useGetDraftAttachments(uuid: string | null) {
  return useQuery({
    queryKey: ["draft-attachments", uuid],
    queryFn: () =>
      apiGet<DraftAttachmentsResponse>(`/drafts/${uuid}/attachments`),
    enabled: !!uuid,
  });
}
