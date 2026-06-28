/** A contact stored in the user's address book. */
export type Contact = {
  id: string;
  email: string;
  name: string;
  company: string;
  notes: string;
  is_favorite: boolean;
  last_contacted: string | null;
  contact_count: number;
  /** How the contact was created: "manual", "import", or "email". */
  source: string;
  created_at: string;
  updated_at: string;
};

/** Response from GET /api/contacts. */
export type ContactsResponse = {
  contacts: Contact[];
  total_count: number;
};

/** A contact group (mailing list / label). */
export type ContactGroup = {
  id: string;
  name: string;
  member_count: number;
  created_at: string;
  updated_at: string;
};
