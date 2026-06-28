"use client";

import { useCallback } from "react";
import { Code2, FileText, Loader2, Monitor, Moon, Search, Sun } from "lucide-react";
import { toast } from "sonner";
import {
  useDisplayPreferences,
  useUpdateDisplayPreferences,
  parseMobileNavTabs,
} from "@/hooks/useDisplayPreferences";
import { runThemeSpreadTransition } from "@/lib/motion/theme-spread";
import { useUiStore } from "@/stores/useUiStore";
import type { ThemeMode } from "@/stores/useUiStore";
import type { AnimationMode } from "@/lib/motion/config";
import { cn } from "@/lib/utils";

function SegmentedControl<T extends string>({
  label,
  description,
  value,
  options,
  onChange,
}: {
  label: string;
  description: string;
  value: T;
  options: { value: T; label: string; icon?: React.ReactNode }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="flex items-center justify-between rounded-lg border border-border p-4">
      <div>
        <div className="text-sm font-medium">{label}</div>
        <p className="mt-0.5 text-xs text-muted-foreground">{description}</p>
      </div>
      <div className="flex rounded-lg border border-border bg-muted/50 p-0.5">
        {options.map((opt) => (
          <button
            key={opt.value}
            type="button"
            onClick={() => onChange(opt.value)}
            className={cn(
              "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors",
              value === opt.value
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground",
            )}
          >
            {opt.icon}
            {opt.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export function DisplaySettings() {
  const { data: prefs, isLoading } = useDisplayPreferences();
  const updatePrefs = useUpdateDisplayPreferences();
  const setDensity = useUiStore((s) => s.setDensity);
  const setTheme = useUiStore((s) => s.setTheme);
  const setComposeFormat = useUiStore((s) => s.setComposeFormat);
  const effectiveAnimationMode = useUiStore((s) => s.effectiveAnimationMode);

  const handleDensityChange = useCallback(
    (density: "compact" | "comfortable") => {
      setDensity(density);
      updatePrefs.mutate(
        { density },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [setDensity, updatePrefs],
  );

  const handleThemeChange = useCallback(
    (theme: ThemeMode) => {
      runThemeSpreadTransition({
        mode: effectiveAnimationMode,
        trigger: "explicit",
        applyTheme: () => setTheme(theme),
        nextTheme: theme,
      });
      updatePrefs.mutate(
        { theme },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [effectiveAnimationMode, setTheme, updatePrefs],
  );

  const handleComposeFormatChange = useCallback(
    (compose_format: "html" | "text") => {
      setComposeFormat(compose_format);
      updatePrefs.mutate(
        { compose_format },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [setComposeFormat, updatePrefs],
  );

  const handleAnimationModeChange = useCallback(
    (animation_mode: AnimationMode) => {
      updatePrefs.mutate(
        { animation_mode },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
    },
    [updatePrefs],
  );

  const handleReset = useCallback(() => {
    setDensity("comfortable");
    setTheme("system");
    setComposeFormat("html");
    updatePrefs.mutate(
      {
        density: "comfortable",
        theme: "system",
        language: "en",
        compose_format: "html",
        animation_mode: null,
        deep_index: false,
        mobile_nav_tabs: null,
        mobile_compose: null,
      },
      { onError: (e) => toast.error(`Failed to reset: ${e.message}`) },
    );
  }, [setDensity, setTheme, setComposeFormat, updatePrefs]);

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 animate-spin" />
        Loading display settings...
      </div>
    );
  }

  if (!prefs) return null;

  return (
    <div className="max-w-2xl space-y-6">
      <div>
        <h2 className="text-base font-semibold">Display</h2>
        <p className="mt-0.5 text-sm text-muted-foreground">
          Customize the appearance and layout of the app.
        </p>
      </div>

      <div className="space-y-3">
        <SegmentedControl
          label="Density"
          description="Controls the spacing of the message list"
          value={prefs.density}
          options={[
            { value: "compact", label: "Compact" },
            { value: "comfortable", label: "Comfortable" },
          ]}
          onChange={handleDensityChange}
        />

        <SegmentedControl
          label="Theme"
          description="Choose light, dark, or match your system"
          value={prefs.theme}
          options={[
            { value: "light", label: "Light", icon: <Sun className="size-3.5" /> },
            { value: "dark", label: "Dark", icon: <Moon className="size-3.5" /> },
            { value: "system", label: "System", icon: <Monitor className="size-3.5" /> },
          ]}
          onChange={handleThemeChange}
        />

        <SegmentedControl
          label="Compose format"
          description="Default format for new emails"
          value={prefs.compose_format}
          options={[
            { value: "html", label: "HTML", icon: <Code2 className="size-3.5" /> },
            { value: "text", label: "Plain text", icon: <FileText className="size-3.5" /> },
          ]}
          onChange={handleComposeFormatChange}
        />

        <SegmentedControl
          label="Animations"
          description="Choose how much motion to use in the interface"
          value={prefs.animation_mode ?? "medium"}
          options={[
            { value: "rich", label: "Rich" },
            { value: "medium", label: "Medium" },
            { value: "subtle", label: "Subtle" },
            { value: "off", label: "Off" },
          ]}
          onChange={handleAnimationModeChange}
        />

      </div>

      <div className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">Composing</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Options for writing and sending messages.
          </p>
        </div>
        <SegmentedControl
          label="Undo send delay"
          description="Window to cancel a send after clicking Send"
          value={String(prefs.undo_send_delay ?? 5) as "0" | "5" | "10" | "30"}
          options={[
            { value: "0", label: "None" },
            { value: "5", label: "5s" },
            { value: "10", label: "10s" },
            { value: "30", label: "30s" },
          ]}
          onChange={(v) =>
            updatePrefs.mutate(
              { undo_send_delay: Number(v) },
              { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
            )
          }
        />
      </div>

      <div className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">Search</h3>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Controls how messages are indexed for search.
          </p>
        </div>
        <SegmentedControl
          label="Search indexing"
          description="Full text downloads message bodies for search (uses more bandwidth)"
          value={prefs.deep_index ? "on" : "off"}
          options={[
            { value: "off", label: "Headers only", icon: <Search className="size-3.5" /> },
            { value: "on", label: "Full text", icon: <Search className="size-3.5" /> },
          ]}
          onChange={(v) =>
            updatePrefs.mutate(
              { deep_index: v === "on" },
              { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
            )
          }
        />
      </div>

      <MobileNavSection prefs={prefs} updatePrefs={updatePrefs} />

      <button
        type="button"
        onClick={handleReset}
        className="text-sm text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
      >
        Reset to defaults
      </button>
    </div>
  );
}

const OPTIONAL_TABS = ["calendar", "contacts", "search", "compose"] as const;
type OptionalTab = (typeof OPTIONAL_TABS)[number];

function MobileNavSection({
  prefs,
  updatePrefs,
}: {
  prefs: ReturnType<typeof useDisplayPreferences>["data"];
  updatePrefs: ReturnType<typeof useUpdateDisplayPreferences>;
}) {
  if (!prefs) return null;

  const enabledTabs = parseMobileNavTabs(prefs.mobile_nav_tabs);
  const composeMode = prefs.mobile_compose ?? "fab";

  const handleTabToggle = (tab: OptionalTab) => {
    const isCompose = tab === "compose";
    if (isCompose) {
      // Toggle compose in tabs vs FAB
      const newMode = composeMode === "tab" ? "fab" : "tab";
      updatePrefs.mutate(
        { mobile_compose: newMode },
        { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
      );
      return;
    }

    const next = enabledTabs.includes(tab)
      ? enabledTabs.filter((t) => t !== tab)
      : [...enabledTabs, tab];

    updatePrefs.mutate(
      { mobile_nav_tabs: JSON.stringify(next) },
      { onError: (e) => toast.error(`Failed to update: ${e.message}`) },
    );
  };

  // Max 3 optional tabs (compose counts if it's in tabs)
  const optionalCount = enabledTabs.filter((t) => t !== "compose").length + (composeMode === "tab" ? 1 : 0);
  const atMax = optionalCount >= 3;

  return (
    <div className="space-y-3">
      <div>
        <h3 className="text-sm font-semibold">Mobile navigation</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Configure the bottom navigation on small screens.
        </p>
      </div>

      <div className="rounded-lg border border-border p-4">
        <div className="mb-3">
          <div className="text-sm font-medium">Tab items</div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Mail is always first. Choose up to 3 additional tabs.
          </p>
        </div>
        <div className="divide-y divide-border">
          {OPTIONAL_TABS.map((tab) => {
            const isCompose = tab === "compose";
            const checked = isCompose ? composeMode === "tab" : enabledTabs.includes(tab);
            const disabled = !checked && atMax;

            return (
              <button
                key={tab}
                type="button"
                role="checkbox"
                aria-checked={checked}
                disabled={disabled}
                onClick={() => handleTabToggle(tab)}
                className={cn(
                  "flex w-full items-center gap-3 py-2 text-left transition-colors",
                  disabled ? "cursor-not-allowed opacity-40" : "cursor-pointer",
                )}
              >
                <span className={cn(
                  "flex size-4 shrink-0 items-center justify-center rounded border transition-colors",
                  checked
                    ? "border-primary bg-primary text-primary-foreground"
                    : "border-muted-foreground/40 bg-transparent",
                  !disabled && !checked && "hover:border-primary",
                )}>
                  {checked && (
                    <svg className="size-3" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M2.5 6l2.5 2.5 4.5-5" />
                    </svg>
                  )}
                </span>
                <span className="text-sm capitalize">{tab}</span>
                {isCompose && (
                  <span className="ml-auto text-xs text-muted-foreground">removes floating button</span>
                )}
              </button>
            );
          })}
        </div>
      </div>

    </div>
  );
}
