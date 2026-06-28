"use client";

import { useState } from "react";
import { Plus, Trash2, Pencil, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { useFilters, useCreateFilter, useUpdateFilter, useDeleteFilter } from "@/hooks/useFilters";
import { useFolders } from "@/hooks/useFolders";
import { useTags } from "@/hooks/useTags";
import type { FilterRule, FilterCondition, FilterAction, CreateFilterRule } from "@/types/filter";
import { cn } from "@/lib/utils";

const FIELD_LABELS: Record<string, string> = {
  from: "From",
  to: "To",
  subject: "Subject",
  has_attachment: "Has attachment",
};

const OP_LABELS: Record<string, string> = {
  contains: "contains",
  equals: "equals",
  starts_with: "starts with",
};

const ACTION_LABELS: Record<string, string> = {
  move: "Move to folder",
  mark_read: "Mark as read",
  delete: "Delete",
  tag: "Tag with",
};

function ConditionRow({
  condition,
  onChange,
  onRemove,
}: {
  condition: FilterCondition;
  onChange: (c: FilterCondition) => void;
  onRemove: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <select
        value={condition.field}
        onChange={(e) => onChange({ ...condition, field: e.target.value as FilterCondition["field"] })}
        className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
      >
        <option value="from">From</option>
        <option value="to">To</option>
        <option value="subject">Subject</option>
        <option value="has_attachment">Has attachment</option>
      </select>

      {condition.field !== "has_attachment" && (
        <>
          <select
            value={condition.op}
            onChange={(e) => onChange({ ...condition, op: e.target.value as FilterCondition["op"] })}
            className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
          >
            <option value="contains">contains</option>
            <option value="equals">equals</option>
            <option value="starts_with">starts with</option>
          </select>
          <input
            type="text"
            value={condition.value}
            onChange={(e) => onChange({ ...condition, value: e.target.value })}
            placeholder="value"
            className="flex-1 min-w-32 rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
          />
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
  const [conditions, setConditions] = useState<FilterCondition[]>(
    initial?.conditions ?? [{ field: "from", op: "contains", value: "" }],
  );
  const [action, setAction] = useState<FilterAction>(
    initial?.action ?? { action_type: "move", action_value: null },
  );

  const addCondition = () =>
    setConditions((prev) => [...prev, { field: "from", op: "contains", value: "" }]);

  const updateCondition = (i: number, c: FilterCondition) =>
    setConditions((prev) => prev.map((x, idx) => (idx === i ? c : x)));

  const removeCondition = (i: number) =>
    setConditions((prev) => prev.filter((_, idx) => idx !== i));

  const handleSave = () => {
    if (!name.trim()) {
      toast.error("Rule name is required");
      return;
    }
    if (conditions.length === 0) {
      toast.error("At least one condition is required");
      return;
    }
    if ((action.action_type === "move" || action.action_type === "tag") && !action.action_value) {
      toast.error(`${ACTION_LABELS[action.action_type]} requires a selection`);
      return;
    }

    const payload: CreateFilterRule = { name, enabled, conditions, action };

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

        <div className="space-y-5 p-5">
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

          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-sm font-medium">Conditions (all must match)</span>
              <button
                type="button"
                onClick={addCondition}
                className="flex items-center gap-1 text-xs text-primary hover:underline"
              >
                <Plus className="size-3" /> Add condition
              </button>
            </div>
            <div className="space-y-2">
              {conditions.map((c, i) => (
                <ConditionRow
                  key={i}
                  condition={c}
                  onChange={(updated) => updateCondition(i, updated)}
                  onRemove={() => removeCondition(i)}
                />
              ))}
            </div>
          </div>

          <div>
            <div className="text-sm font-medium mb-2">Action</div>
            <div className="flex flex-wrap items-center gap-2">
              <select
                value={action.action_type}
                onChange={(e) =>
                  setAction({ action_type: e.target.value as FilterAction["action_type"], action_value: null })
                }
                className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
              >
                <option value="move">Move to folder</option>
                <option value="mark_read">Mark as read</option>
                <option value="delete">Delete</option>
                <option value="tag">Tag with</option>
              </select>

              {action.action_type === "move" && (
                <select
                  value={action.action_value ?? ""}
                  onChange={(e) => setAction({ ...action, action_value: e.target.value || null })}
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
                  onChange={(e) => setAction({ ...action, action_value: e.target.value || null })}
                  className="rounded-md border border-border bg-background px-2 py-1.5 text-sm focus:outline-none focus:ring-2 focus:ring-primary/50"
                >
                  <option value="">Select tag...</option>
                  {tags.map((t) => (
                    <option key={t.id} value={t.id}>{t.name}</option>
                  ))}
                </select>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2">
            <input
              id="rule-enabled"
              type="checkbox"
              checked={enabled}
              onChange={(e) => setEnabled(e.target.checked)}
              className="rounded border-border"
            />
            <label htmlFor="rule-enabled" className="text-sm">Enabled</label>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-border px-5 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-md px-4 py-2 text-sm text-muted-foreground hover:text-foreground"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={handleSave}
            disabled={isPending}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {isPending ? "Saving..." : "Save rule"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ruleDescription(rule: FilterRule): string {
  const conds = rule.conditions
    .map((c) => {
      if (c.field === "has_attachment") return "has attachment";
      return `${FIELD_LABELS[c.field] ?? c.field} ${OP_LABELS[c.op] ?? c.op} "${c.value}"`;
    })
    .join(" AND ");
  const action = ACTION_LABELS[rule.action.action_type] ?? rule.action.action_type;
  const target = rule.action.action_value ? ` - ${rule.action.action_value}` : "";
  return `${conds} → ${action}${target}`;
}

export function FiltersSettings() {
  const { data, isLoading } = useFilters();
  const { data: foldersData } = useFolders();
  const { data: tagsData } = useTags();
  const deleteFilter = useDeleteFilter();
  const updateFilter = useUpdateFilter();

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
        <button
          type="button"
          onClick={() => setDialogMode({ type: "create" })}
          className="flex items-center gap-1.5 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          <Plus className="size-4" />
          New rule
        </button>
      </div>

      {rules.length === 0 ? (
        <div className="rounded-lg border border-dashed border-border p-8 text-center">
          <p className="text-sm text-muted-foreground">No filter rules yet.</p>
          <button
            type="button"
            onClick={() => setDialogMode({ type: "create" })}
            className="mt-2 text-sm text-primary hover:underline"
          >
            Create your first rule
          </button>
        </div>
      ) : (
        <div className="divide-y divide-border rounded-lg border border-border">
          {rules.map((rule) => (
            <div key={rule.id} className="flex items-center gap-3 px-4 py-3">
              <input
                type="checkbox"
                checked={rule.enabled}
                onChange={() => handleToggleEnabled(rule)}
                className="rounded border-border"
                aria-label={`${rule.enabled ? "Disable" : "Enable"} rule ${rule.name}`}
              />
              <div className="flex-1 min-w-0">
                <div className="text-sm font-medium truncate">{rule.name}</div>
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
                className={cn(
                  "text-muted-foreground hover:text-destructive",
                  deleteFilter.isPending && "opacity-50 pointer-events-none",
                )}
                aria-label="Delete rule"
              >
                <Trash2 className="size-4" />
              </button>
            </div>
          ))}
        </div>
      )}

      {dialogMode && (
        <FilterRuleDialog
          mode={dialogMode}
          folders={folders}
          tags={tags}
          onClose={() => setDialogMode(null)}
        />
      )}
    </div>
  );
}
