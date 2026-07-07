import type { MessageHeader } from "@/types/message";

export type Folder = {
  name: string;
  delimiter: string | null;
  attributes: string[];
  is_subscribed: boolean;
  total_count: number;
  unread_count: number;
  /** Top 20 most recent messages, pre-seeded into the client cache on load. */
  recent_messages: MessageHeader[];
};

export type FoldersResponse = {
  folders: Folder[];
};
