import type { MessageTag } from "@/types/tag";
import type { PgpMessageStatus } from "@/types/pgp";

/** A parsed email address from the API. */
export type EmailAddress = {
  name: string | null;
  address: string;
};

/**
 * A message summary returned by the list API. Represents the latest message in
 * a thread, with thread_count and unread_count aggregates for UI grouping.
 */
export type MessageHeader = {
  uid: number;
  folder: string;
  subject: string;
  from_address: string;
  from_name: string;
  to_addresses: string;
  cc_addresses: string;
  date: string;
  flags: string;
  size: number;
  has_attachments: boolean;
  snippet: string;
  reaction: string | null;
  tags: MessageTag[];
  /** Total number of messages in this thread within the folder. */
  thread_count: number;
  /** Number of unread messages in this thread. */
  unread_count: number;
};

/** Full message detail returned when opening a message. */
export type MessageDetail = {
  uid: number;
  folder: string;
  subject: string;
  from_address: string;
  from_name: string;
  to_addresses: EmailAddress[];
  cc_addresses: EmailAddress[];
  date: string;
  flags: string[];
  html: string | null;
  text: string | null;
  email_theme?: 'light' | 'dark' | 'transparent' | 'adaptive';
  raw_headers: string;
  attachments: Attachment[];
  thread: MessageHeader[];
  pgp_status: PgpMessageStatus | null;
};

/** Email attachment metadata. */
export type Attachment = {
  id: string;
  filename: string | null;
  content_type: string;
  size: number;
  content_id: string | null;
};

/** Paginated message list response. */
export type MessagesResponse = {
  messages: MessageHeader[];
  total_count: number;
  page: number;
  per_page: number;
  syncing?: boolean;
};

/** A search result item (flat, not threaded). */
export type SearchResultItem = {
  uid: number;
  folder: string;
  score: number;
  subject: string;
  from_address: string;
  from_name: string;
  to_addresses: string;
  date: string;
  flags: string;
  has_attachments: boolean;
  snippet: string;
};

/** Search response envelope. */
export type SearchResponse = {
  results: SearchResultItem[];
  total_count: number;
  query: string;
};
