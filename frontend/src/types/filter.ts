/** A single condition in a filter rule. */
export type FilterCondition = {
  /** Message field to match against: "from" | "to" | "subject" | "has_attachment". */
  field: "from" | "to" | "subject" | "has_attachment";
  /** Match operation: "contains" | "equals" | "starts_with". */
  op: "contains" | "equals" | "starts_with";
  /** Value to match (ignored for has_attachment). */
  value: string;
};

/** The action to take when a rule matches. */
export type FilterAction = {
  /** Action type: "move" | "mark_read" | "delete" | "tag". */
  action_type: "move" | "mark_read" | "delete" | "tag";
  /** Required for "move" (folder name) and "tag" (tag id). Null otherwise. */
  action_value: string | null;
};

/** A complete filter rule as returned by the API. */
export type FilterRule = {
  id: string;
  name: string;
  enabled: boolean;
  /** Lower priority runs first. */
  priority: number;
  /** All conditions must match (AND logic). */
  conditions: FilterCondition[];
  action: FilterAction;
  created_at: string;
  updated_at: string;
};

/** Body for POST /api/filters. */
export type CreateFilterRule = {
  name: string;
  enabled?: boolean;
  priority?: number;
  conditions: FilterCondition[];
  action: FilterAction;
};

/** Body for PUT /api/filters/{id}. All fields optional. */
export type UpdateFilterRule = {
  name?: string;
  enabled?: boolean;
  priority?: number;
  conditions?: FilterCondition[];
  action?: FilterAction;
};

/** Response shape for GET /api/filters. */
export type FiltersResponse = {
  rules: FilterRule[];
};
