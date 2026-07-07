/** A single condition in a filter rule. */
export type FilterCondition = {
  /**
   * Message field to match against.
   * Boolean fields (has_attachment, is_reply): op/value ignored.
   * Size field: uses greater_than / less_than ops with byte value.
   */
  field: "from" | "to" | "cc" | "subject" | "body" | "has_attachment" | "is_reply" | "size";
  /**
   * Comparison operator.
   * String fields: contains | not_contains | equals | not_equals | starts_with | ends_with | matches_regex
   * Size field: greater_than | less_than
   * Boolean fields: ignored
   */
  op: "contains" | "not_contains" | "equals" | "not_equals" | "starts_with" | "ends_with" | "matches_regex" | "greater_than" | "less_than";
  /** Value to match. Empty for boolean fields. Bytes for size field. */
  value: string;
};

/** A single action to take when a rule matches. */
export type FilterAction = {
  /** Action type. */
  action_type: "move" | "mark_read" | "mark_starred" | "delete" | "tag" | "forward";
  /**
   * - move: target folder name
   * - tag: tag id
   * - forward: destination email address
   * - mark_read / mark_starred / delete: null
   */
  action_value: string | null;
};

/** A complete filter rule as returned by the API. */
export type FilterRule = {
  id: string;
  name: string;
  enabled: boolean;
  /** Lower priority runs first. */
  priority: number;
  conditions: FilterCondition[];
  /** "all" = AND (every condition must match), "any" = OR (at least one must match). */
  match_mode: "all" | "any";
  /** Actions executed in order when the rule matches. */
  actions: FilterAction[];
  /** When true, no further rules are evaluated after this one matches. */
  stop_processing: boolean;
  created_at: string;
  updated_at: string;
};

/** Body for POST /api/filters. */
export type CreateFilterRule = {
  name: string;
  enabled?: boolean;
  priority?: number;
  conditions: FilterCondition[];
  match_mode?: "all" | "any";
  actions: FilterAction[];
  stop_processing?: boolean;
};

/** Body for PUT /api/filters/{id}. All fields optional. */
export type UpdateFilterRule = {
  name?: string;
  enabled?: boolean;
  priority?: number;
  conditions?: FilterCondition[];
  match_mode?: "all" | "any";
  actions?: FilterAction[];
  stop_processing?: boolean;
};

/** Body for PUT /api/filters/reorder. */
export type ReorderFiltersBody = {
  /** Filter rule IDs in the desired order. */
  ids: string[];
};

/** Response shape for GET /api/filters and PUT /api/filters/reorder. */
export type FiltersResponse = {
  rules: FilterRule[];
};

/** Response shape for POST /api/filters/apply. */
export type ApplyFiltersResponse = {
  applied: number;
  errors: string[];
};
