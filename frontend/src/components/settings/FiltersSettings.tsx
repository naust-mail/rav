"use client";

import { useState } from "react";
import { Plus, Trash2, Pencil, Loader2, ChevronUp, ChevronDown, Play } from "lucide-react";
import { toast } from "sonner";
import { useFilters, useCreateFilter, useUpdateFilter, useDeleteFilter, useReorderFilters, useApplyFilters } from "@/hooks/useFilters";
import { useFolders } from "@/hooks/useFolders";
import { useTags } from "@/hooks/useTags";
import type { FilterRule, FilterCondition, FilterAction, CreateFilterRule } from "@/types/filter";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Label maps
// ---------------------------------------------------------------------------

const FIELD_LABELS: Record<string, string> = {
  from: "From",
  to: "To",
  cc: "CC",
  subject: "Subject",
  body: "Body (preview)",
  has_attachment: "Has attachment",
  is_reply: "Is a reply",
  size: "Size",
};

const OP_LABELS: Record<string, string> = {
  contains: "contains",
  not_contains: "does not contain",
  equals: "equals",
  not_equals: "does not equal",
  starts_with: "starts with",
  ends_with: "ends with",
  matches_regex: "matches regex",
  greater_than: "is greater than",
  less_than: "is less than",
};

const ACTION_LABELS: Record<string, string> = {
  move: "Move to folder",
  mark_read: "Mark as read",
  mark_starred: "Mark as starred",
  delete: "Delete",
  tag: "Tag with",
  forward: "Forward to",
};

// Fields that have no op/value inputs (boolean checks)
const BOOLEAN_FIELDS = new Set(["has_attachment", "is_reply"]);
// Fields that use numeric ops only
const NUMERIC_FIELDS = new Set(["size"]);

// ---------------------------------------------------------------------------
// Size value helpers (store bytes, display in KB/MB)
// ---------------------------------------------------------------------------

type SizeUnit = "KB" | "MB";

function bytesToDisplay(bytes: string): { value: string; unit: SizeUnit } {
  const n = parseInt(bytes, 10) || 0;
  if (n >= 1_000_000 && n % 1_000_000 === 0) return { value: String(n / 1_000_000), unit: "MB" };
  return { value: String(Math.round(n / 1024) || 1), unit: "KB" };
}

function displayToBytes(value: string, unit: SizeUnit): string {
  const n = parseInt(value, 10) || 0;
  return String(unit === "MB" ? n * 1_000_000 : n * 1024);
}

// ---------------------------------------------------------------------------
// ConditionRow
// ---------------------------------------------------------------------------

function ConditionRow({
  condition,
  onChange,
  onRemove,
}: {
  condition: FilterCondition;
  onChange: (c: FilterCondition) => void;
  onRemove: () => void;
}) {
  const isBoolean = BOOLEAN_FIELDS.has(condition.field);
  const isNumeric = NUMERIC_FIELDS.has(condition.field);

  // Local size display state
  const sizeDisplay = isNumeric ? bytesToDisplay(condition.value || "0") : null;

  return (
    <div className="flex flex-wrap items-center gap-2">
      {/* Field selector */}
      <select
        value={condition.field}
        onChange={(e) => {
          const field = e.target.value as FilterCondition["field"];
          const op: FilterCondition["op"] = NUMERIC_FIELDS.has(field)
            ? "greater_than"
            : BOOLEAN_FIELDS.has(field)
              ? "contains"
              : "contains";
          onChange({ field, op, value: NUMERIC_FIELDS.has(field) ? "1048576" : "" });
        }}
        className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
      >
        <option value="from">From</option>
        <option value="to">To</option>
        <option value="cc">CC</option>
        <option value="subject">Subject</option>
        <option value="body">Body (preview)</option>
        <option value="has_attachment">Has attachment</option>
        <option value="is_reply">Is a reply</option>
        <option value="size">Size</option>
      </select>

      {/* Operator - string fields only */}
      {!isBoolean && !isNumeric && (
        <select
          value={condition.op}
          onChange={(e) => onChange({ ...condition, op: e.target.value as FilterCondition["op"] })}
          className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        >
          <option value="contains">contains</option>
          <option value="not_contains">does not contain</option>
          <option value="equals">equals</option>
          <option value="not_equals">does not equal</option>
          <option value="starts_with">starts with</option>
          <option value="ends_with">ends with</option>
          <option value="matches_regex">matches regex</option>
        </select>
      )}

      {/* Operator - size field */}
      {isNumeric && (
        <select
          value={condition.op}
          onChange={(e) => onChange({ ...condition, op: e.target.value as FilterCondition["op"] })}
          className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        >
          <option value="greater_than">is greater than</option>
          <option value="less_than">is less than</option>
        </select>
      )}

      {/* Value - string fields */}
      {!isBoolean && !isNumeric && (
        <input
          type="text"
          value={condition.value}
          onChange={(e) => onChange({ ...condition, value: e.target.value })}
          placeholder={condition.op === "matches_regex" ? "regex pattern" : "value"}
          className="flex-1 min-w-32 rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        />
      )}

      {/* Value - size field: number + unit */}
      {isNumeric && sizeDisplay && (
        <>
          <input
            type="number"
            min="1"
            value={sizeDisplay.value}
            onChange={(e) => {
              const bytes = displayToBytes(e.target.value, sizeDisplay.unit);
              onChange({ ...condition, value: bytes });
            }}
            className="w-20 rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
          />
          <select
            value={sizeDisplay.unit}
            onChange={(e) => {
              const unit = e.target.value as SizeUnit;
              const bytes = displayToBytes(sizeDisplay.value, unit);
              onChange({ ...condition, value: bytes });
            }}
            className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
          >
            <option value="KB">KB</option>
            <option value="MB">MB</option>
          </select>
        </>
      )}

      <button
        type="button"
        onClick={onRemove}
        className="text-muted-foreground hover:text-destructive"
        aria-label="Remove condition"
      >
        <Trash2 className="size-3.5" />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ActionRow
// ---------------------------------------------------------------------------

function ActionRow({
  action,
  folders,
  tags,
  onChange,
  onRemove,
  canRemove,
}: {
  action: FilterAction;
  folders: string[];
  tags: { id: string; name: string }[];
  onChange: (a: FilterAction) => void;
  onRemove: () => void;
  canRemove: boolean;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <select
        value={action.action_type}
        onChange={(e) =>
          onChange({ action_type: e.target.value as FilterAction["action_type"], action_value: null })
        }
        className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
      >
        <option value="move">Move to folder</option>
        <option value="mark_read">Mark as read</option>
        <option value="mark_starred">Mark as starred</option>
        <option value="delete">Delete</option>
        <option value="tag">Tag with</option>
        <option value="forward">Forward to</option>
      </select>

      {action.action_type === "move" && (
        <select
          value={action.action_value ?? ""}
          onChange={(e) => onChange({ ...action, action_value: e.target.value || null })}
          className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        >
          <option value="">Select folder...</option>
          {folders.map((f) => (
            <option key={f} value={f}>{f}</option>
          ))}
        </select>
      )}

      {action.action_type === "tag" && (
        <select
          value={action.action_value ?? ""}
          onChange={(e) => onChange({ ...action, action_value: e.target.value || null })}
          className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        >
          <option value="">Select tag...</option>
          {tags.map((t) => (
            <option key={t.id} value={t.id}>{t.name}</option>
          ))}
        </select>
      )}

      {action.action_type === "forward" && (
        <input
          type="email"
          value={action.action_value ?? ""}
          onChange={(e) => onChange({ ...action, action_value: e.target.value || null })}
          placeholder="email@example.com"
          className="flex-1 min-w-44 rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
        />
      )}

      {canRemove && (
        <button
          type="button"
          onClick={onRemove}
          className="text-muted-foreground hover:text-destructive"
          aria-label="Remove action"
        >
          <Trash2 className="size-3.5" />
        </button>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

type DialogMode = { type: "create" } | { type: "edit"; rule: FilterRule };

function FilterRuleDialog({
  mode,
  folders,
  tags,
  onClose,
}: {
  mode: DialogMode;
  folders: string[];
  tags: { id: string; name: string }[];
  onClose: () => void;
}) {
  const createFilter = useCreateFilter();
  const updateFilter = useUpdateFilter();

  const initial = mode.type === "edit" ? mode.rule : null;

  const [name, setName] = useState(initial?.name ?? "");
  const [enabled, setEnabled] = useState(initial?.enabled ?? true);
  const [matchMode, setMatchMode] = useState<"all" | "any">(initial?.match_mode ?? "all");
  const [stopProcessing, setStopProcessing] = useState(initial?.stop_processing ?? false);
  const [conditions, setConditions] = useState<FilterCondition[]>(
    initial?.conditions ?? [{ field: "from", op: "contains", value: "" }],
  );
  const [actions, setActions] = useState<FilterAction[]>(
    initial?.actions ?? [{ action_type: "move", action_value: null }],
  );

  const addCondition = () =>
    setConditions((p) => [...p, { field: "from", op: "contains", value: "" }]);

  const updateCondition = (i: number, c: FilterCondition) =>
    setConditions((p) => p.map((x, idx) => (idx === i ? c : x)));

  const removeCondition = (i: number) =>
    setConditions((p) => p.filter((_, idx) => idx !== i));

  const addAction = () =>
    setActions((p) => [...p, { action_type: "mark_read", action_value: null }]);

  const updateAction = (i: number, a: FilterAction) =>
    setActions((p) => p.map((x, idx) => (idx === i ? a : x)));

  const removeAction = (i: number) =>
    setActions((p) => p.filter((_, idx) => idx !== i));

  const handleSave = () => {
    if (!name.trim()) { toast.error("Rule name is required"); return; }
    if (conditions.length === 0) { toast.error("At least one condition is required"); return; }
    for (const action of actions) {
      if ((action.action_type === "move" || action.action_type === "tag") && !action.action_value) {
        toast.error(`${ACTION_LABELS[action.action_type]} requires a selection`);
        return;
      }
      if (action.action_type === "forward") {
        const addr = action.action_value ?? "";
        if (!addr || !addr.includes("@")) {
          toast.error("Forward action requires a valid email address");
          return;
        }
      }
    }

    const payload: CreateFilterRule = { name, enabled, match_mode: matchMode, stop_processing: stopProcessing, conditions, actions };

    if (mode.type === "create") {
      createFilter.mutate(payload, {
        onSuccess: () => { toast.success("Filter rule created"); onClose(); },
        onError: (e) => toast.error(`Failed: ${e.message}`),
      });
    } else {
      updateFilter.mutate({ id: mode.rule.id, data: payload }, {
        onSuccess: () => { toast.success("Filter rule updated"); onClose(); },
        onError: (e) => toast.error(`Failed: ${e.message}`),
      });
    }
  };

  const isPending = createFilter.isPending || updateFilter.isPending;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-xl border border-border bg-background shadow-xl">
        <div className="border-b border-border px-5 py-4">
          <h2 className="text-base font-semibold">
            {mode.type === "create" ? "New filter rule" : "Edit filter rule"}
          </h2>
        </div>

        <div className="space-y-5 p-5 max-h-[70vh] overflow-y-auto">
          {/* Name */}
          <div>
            <label className="block text-sm font-medium mb-1" htmlFor="rule-name">Name</label>
            <input
              id="rule-name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Rule name"
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
            />
          </div>

          {/* Conditions */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium">Conditions</span>
                <div className="flex items-center rounded-md border border-border overflow-hidden text-xs">
                  <button
                    type="button"
                    onClick={() => setMatchMode("all")}
                    className={cn(
                      "px-2 py-1 transition-colors",
                      matchMode === "all" ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground",
                    )}
                  >All</button>
                  <button
                    type="button"
                    onClick={() => setMatchMode("any")}
                    className={cn(
                      "px-2 py-1 transition-colors",
                      matchMode === "any" ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground",
                    )}
                  >Any</button>
                </div>
                <span className="text-xs text-muted-foreground">must match</span>
              </div>
              <button type="button" onClick={addCondition} className="flex items-center gap-1 text-xs text-primary hover:underline">
                <Plus className="size-3" /> Add
              </button>
            </div>
            <div className="space-y-2">
              {conditions.map((c, i) => (
                <ConditionRow key={i} condition={c} onChange={(u) => updateCondition(i, u)} onRemove={() => removeCondition(i)} />
              ))}
            </div>
          </div>

          {/* Actions */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-sm font-medium">Actions</span>
              <button type="button" onClick={addAction} className="flex items-center gap-1 text-xs text-primary hover:underline">
                <Plus className="size-3" /> Add
              </button>
            </div>
            <div className="space-y-2">
              {actions.map((a, i) => (
                <ActionRow key={i} action={a} folders={folders} tags={tags}
                  onChange={(u) => updateAction(i, u)} onRemove={() => removeAction(i)} canRemove={actions.length > 1} />
              ))}
            </div>
          </div>

          {/* Options */}
          <div className="space-y-2">
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="rounded border-border" />
              Enabled
            </label>
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input type="checkbox" checked={stopProcessing} onChange={(e) => setStopProcessing(e.target.checked)} className="rounded border-border" />
              <span>
                Stop processing further rules
                <span className="ml-1 text-xs text-muted-foreground">(when this rule matches)</span>
              </span>
            </label>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-border px-5 py-4">
          <button type="button" onClick={onClose} className="rounded-md px-4 py-2 text-sm text-muted-foreground hover:text-foreground">
            Cancel
          </button>
          <button
            type="button" onClick={handleSave} disabled={isPending}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {isPending ? "Saving..." : "Save rule"}
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Rule description
// ---------------------------------------------------------------------------

function ruleDescription(rule: FilterRule): string {
  const sep = rule.match_mode === "any" ? " OR " : " AND ";
  const conds = rule.conditions.map((c) => {
    if (c.field === "has_attachment") return "has attachment";
    if (c.field === "is_reply") return "is a reply";
    if (c.field === "size") {
      const bytes = parseInt(c.value, 10) || 0;
      const display = bytes >= 1_000_000 ? `${bytes / 1_000_000} MB` : `${Math.round(bytes / 1024)} KB`;
      return `size ${OP_LABELS[c.op] ?? c.op} ${display}`;
    }
    return `${FIELD_LABELS[c.field] ?? c.field} ${OP_LABELS[c.op] ?? c.op} "${c.value}"`;
  }).join(sep);

  const actionStr = rule.actions.map((a) => {
    const label = ACTION_LABELS[a.action_type] ?? a.action_type;
    return a.action_value ? `${label} ${a.action_value}` : label;
  }).join(", ");

  return `${conds} -> ${actionStr}`;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function FiltersSettings() {
  const { data, isLoading } = useFilters();
  const { data: foldersData } = useFolders();
  const { data: tagsData } = useTags();
  const deleteFilter = useDeleteFilter();
  const updateFilter = useUpdateFilter();
  const reorderFilters = useReorderFilters();
  const applyFilters = useApplyFilters();

  const [dialogMode, setDialogMode] = useState<DialogMode | null>(null);

  const folders = (foldersData?.folders ?? []).map((f) => f.name);
  const tags = (tagsData?.tags ?? []).map((t) => ({ id: t.id, name: t.name }));

  const handleDelete = (rule: FilterRule) => {
    deleteFilter.mutate(rule.id, {
      onSuccess: () => toast.success(`Deleted rule "${rule.name}"`),
      onError: (e) => toast.error(`Failed: ${e.message}`),
    });
  };

  const handleToggleEnabled = (rule: FilterRule) => {
    updateFilter.mutate(
      { id: rule.id, data: { enabled: !rule.enabled } },
      { onError: (e) => toast.error(`Failed: ${e.message}`) },
    );
  };

  const handleMove = (rules: FilterRule[], index: number, direction: -1 | 1) => {
    const next = index + direction;
    if (next < 0 || next >= rules.length) return;
    const reordered = [...rules];
    [reordered[index], reordered[next]] = [reordered[next], reordered[index]];
    reorderFilters.mutate(
      { ids: reordered.map((r) => r.id) },
      { onError: (e) => toast.error(`Failed to reorder: ${e.message}`) },
    );
  };

  const handleApply = () => {
    applyFilters.mutate(undefined, {
      onSuccess: (data) => {
        if (data.errors.length > 0) {
          toast.warning(`Applied to ${data.applied} message(s) with ${data.errors.length} error(s)`);
        } else {
          toast.success(`Applied rules to ${data.applied} matching message(s)`);
        }
      },
      onError: (e) => toast.error(`Failed: ${e.message}`),
    });
  };

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Loading filters...
      </div>
    );
  }

  const rules = data?.rules ?? [];

  return (
    <div className="max-w-2xl space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-base font-semibold">Filter Rules</h2>
          <p className="mt-0.5 text-sm text-muted-foreground">
            Automatically act on incoming messages based on conditions.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {rules.length > 0 && (
            <button
              type="button"
              onClick={handleApply}
              disabled={applyFilters.isPending}
              className="flex items-center gap-1.5 rounded-md border border-border px-3 py-2 text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
              title="Run all rules against existing inbox messages"
            >
              {applyFilters.isPending ? <Loader2 className="size-4 animate-spin" /> : <Play className="size-4" />}
              Apply to inbox
            </button>
          )}
          <button
            type="button"
            onClick={() => setDialogMode({ type: "create" })}
            className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            <Plus className="size-4" />
            New rule
          </button>
        </div>
      </div>

      {rules.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border p-8 text-center">
          <p className="text-sm text-muted-foreground">No filter rules yet.</p>
          <button type="button" onClick={() => setDialogMode({ type: "create" })} className="mt-2 text-sm text-primary hover:underline">
            Create your first rule
          </button>
        </div>
      ) : (
        <div className="divide-y divide-border rounded-lg border border-border">
          {rules.map((rule, index) => (
            <div key={rule.id} className="flex items-center gap-3 px-4 py-3">
              {/* Reorder buttons */}
              <div className="flex flex-col">
                <button
                  type="button"
                  onClick={() => handleMove(rules, index, -1)}
                  disabled={index === 0 || reorderFilters.isPending}
                  className="text-muted-foreground hover:text-foreground disabled:opacity-25"
                  aria-label="Move rule up"
                >
                  <ChevronUp className="size-3.5" />
                </button>
                <button
                  type="button"
                  onClick={() => handleMove(rules, index, 1)}
                  disabled={index === rules.length - 1 || reorderFilters.isPending}
                  className="text-muted-foreground hover:text-foreground disabled:opacity-25"
                  aria-label="Move rule down"
                >
                  <ChevronDown className="size-3.5" />
                </button>
              </div>

              <input
                type="checkbox"
                checked={rule.enabled}
                onChange={() => handleToggleEnabled(rule)}
                className="rounded border-border"
                aria-label={`${rule.enabled ? "Disable" : "Enable"} rule ${rule.name}`}
              />

              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium truncate">{rule.name}</span>
                  {rule.stop_processing && (
                    <span className="shrink-0 rounded px-1 py-0.5 text-xs bg-muted text-muted-foreground">stops</span>
                  )}
                </div>
                <div className="text-xs text-muted-foreground truncate">{ruleDescription(rule)}</div>
              </div>

              <button
                type="button"
                onClick={() => setDialogMode({ type: "edit", rule })}
                className="text-muted-foreground hover:text-foreground"
                aria-label="Edit rule"
              >
                <Pencil className="size-4" />
              </button>
              <button
                type="button"
                onClick={() => handleDelete(rule)}
                className={cn("text-muted-foreground hover:text-destructive", deleteFilter.isPending && "opacity-50 pointer-events-none")}
                aria-label="Delete rule"
              >
                <Trash2 className="size-4" />
              </button>
            </div>
          ))}
        </div>
      )}

      {dialogMode && (
        <FilterRuleDialog mode={dialogMode} folders={folders} tags={tags} onClose={() => setDialogMode(null)} />
      )}
    </div>
  );
}
